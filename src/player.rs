use super::{Catalog, Error, Session};
use serenity::all::*;
use std::sync::{Arc, atomic::Ordering};
use vozen_music::skip::SkipMode;

pub async fn acknowledge_button(
    http: &Http,
    button: &ComponentInteraction,
) -> serenity::Result<()> {
    button
        .create_response(http, CreateInteractionResponse::Acknowledge)
        .await
}

pub async fn finish_button(
    http: &Http,
    button: &ComponentInteraction,
    result: Result<String, Error>,
) -> serenity::Result<()> {
    let content = match result {
        Err(error) => Some(error.to_string()),
        Ok(message)
            if ["vozen:action:like:", "vozen:action:dislike:"]
                .iter()
                .any(|prefix| button.data.custom_id.starts_with(prefix)) =>
        {
            Some(message)
        }
        _ => None,
    };
    if let Some(content) = content {
        button
            .create_followup(
                http,
                CreateInteractionResponseFollowup::new()
                    .content(content)
                    .ephemeral(true)
                    .allowed_mentions(CreateAllowedMentions::new()),
            )
            .await?;
    }
    Ok(())
}

pub(super) struct Artwork {
    file: std::path::PathBuf,
    url: String,
}

fn attached_cover(file: Option<&std::path::PathBuf>, message: &Message) -> Option<Artwork> {
    let file = file?;
    let name = super::player_artwork::filename(file);
    // Discord may consume an embed upload without listing it in message.attachments.
    let url = message
        .embeds
        .iter()
        .filter_map(|e| e.image.as_ref().map(|i| i.url.as_str()))
        .chain(
            message
                .attachments
                .iter()
                .filter(|a| a.filename == name)
                .map(|a| a.url.as_str()),
        )
        .find(|url| {
            reqwest::Url::parse(url).is_ok_and(|u| {
                u.scheme() == "https"
                    && matches!(
                        u.host_str(),
                        Some("cdn.discordapp.com" | "media.discordapp.net")
                    )
                    && u.path_segments().and_then(|mut parts| parts.next_back())
                        == Some(name.as_str())
            })
        })?;
    Some(Artwork {
        file: file.clone(),
        url: url.to_owned(),
    })
}

pub fn target(id: &str) -> Result<(u64, usize, bool), Error> {
    let parts: Vec<_> = id.split(':').collect();
    if parts.len() != 4 || parts[0] != "vozen" || !matches!(parts[1], "skip" | "previous") {
        return Err("Invalid player control.".into());
    }
    let session = parts[2].parse::<u64>()?;
    let generation = parts[3].parse::<usize>()?;
    if session == 0 || generation == 0 {
        return Err("Invalid player control.".into());
    }
    Ok((session, generation, parts[1] == "previous"))
}

pub fn controls(
    session: &Session,
    generation: usize,
    stopped: bool,
) -> Result<Vec<CreateActionRow>, Error> {
    let paused = session.paused.load(Ordering::SeqCst);
    let looping = session.looping.load(Ordering::SeqCst);
    let volume = session.volume.load(Ordering::SeqCst);
    let action = |name: &str| {
        CreateButton::new(format!(
            "vozen:action:{name}:{}:{generation}",
            session.control_id
        ))
        .style(ButtonStyle::Secondary)
        .disabled(stopped)
    };
    let navigation = |previous: bool| -> Result<CreateButton, Error> {
        let icon = if previous { "⏮" } else { "⏭" };
        Ok(CreateButton::new(format!(
            "vozen:{}:{}:{generation}",
            if previous { "previous" } else { "skip" },
            session.control_id
        ))
        .emoji(ReactionType::Unicode(icon.into()))
        .style(ButtonStyle::Secondary)
        .disabled(
            stopped
                || (previous
                    && session
                        .history
                        .lock()
                        .map_err(|_| "Playback history is unavailable.")?
                        .is_empty()),
        ))
    };
    Ok(vec![
        CreateActionRow::Buttons(vec![
            action("volume-down")
                .emoji(ReactionType::Unicode("🔉".into()))
                .disabled(stopped || volume == 0),
            navigation(true)?,
            action(if paused { "resume" } else { "pause" })
                .emoji(ReactionType::Unicode(if paused { "▶" } else { "⏸" }.into()))
                .style(ButtonStyle::Success),
            navigation(false)?,
            action("volume-up")
                .emoji(ReactionType::Unicode("🔊".into()))
                .disabled(stopped || volume >= 100),
        ]),
        CreateActionRow::Buttons(vec![
            action("like").emoji(ReactionType::Unicode("❤️".into())),
            action("dislike").emoji(ReactionType::Unicode("💔".into())),
            action("loop").label("↻").style(if looping {
                ButtonStyle::Success
            } else {
                ButtonStyle::Secondary
            }),
            action("stop")
                .label("Leave")
                .emoji(ReactionType::Unicode("🚪".into()))
                .style(ButtonStyle::Danger),
        ]),
    ])
}

pub fn vote_progress(session: &Session) -> Result<String, Error> {
    if session
        .policy
        .lock()
        .map_err(|_| "Player settings are unavailable.")?
        .mode
        != SkipMode::Majority
    {
        return Ok(String::new());
    }
    let mut progress = Vec::new();
    for (previous, name) in [(true, "Previous"), (false, "Next")] {
        let votes = session
            .votes_for(previous)
            .lock()
            .map_err(|_| "Skip votes are unavailable.")?;
        if !votes.voters.is_empty() {
            progress.push(format!(
                "{name}: {}/{} votes",
                votes.voters.len(),
                votes.required
            ));
        }
    }
    Ok(progress.join(" · "))
}

pub fn button_volume(action: &str, current: usize) -> Option<i64> {
    match action {
        "volume-down" => Some(current.saturating_sub(10).min(100) as i64),
        "volume-up" => Some(current.saturating_add(10).min(100) as i64),
        _ => None,
    }
}

fn payload(
    catalog: &Catalog,
    session: &Session,
    stopped: bool,
    artwork_url: Option<&str>,
) -> Result<serde_json::Value, Error> {
    let station = &catalog.stations[session.station];
    let generation = session.generation.load(Ordering::SeqCst).max(1);
    let mut components = Vec::new();
    if stopped {
        components.push(serde_json::json!({"type":10,"content":"Playback stopped. Use /play to start a station."}));
    } else {
        let track = &station.tracks[session.current.load(Ordering::SeqCst)];
        if let Some(file) = &track.cover {
            let name = super::player_artwork::filename(file);
            let url = artwork_url
                .map(str::to_owned)
                .unwrap_or_else(|| format!("attachment://{name}"));
            components.push(serde_json::json!({"type":12,"items":[{"media":{"url":url},"description":format!("{} by {}", track.title, track.artist)}]}));
        }
        let safe = |text: &str| {
            serenity::utils::MessageBuilder::new()
                .push_safe(text)
                .build()
        };
        let mut status = format!(
            "{}-# {} · {} · {}%{}",
            if track.cover.is_none() {
                format!("### {}\n{}\n", safe(&track.title), safe(&track.artist))
            } else {
                String::new()
            },
            safe(&station.name),
            if session.paused.load(Ordering::SeqCst) {
                "Paused"
            } else {
                "Playing"
            },
            session.volume.load(Ordering::SeqCst),
            if session.looping.load(Ordering::SeqCst) {
                " · Loop on"
            } else {
                ""
            }
        );
        let votes = vote_progress(session)?;
        if !votes.is_empty() {
            status.push_str(&format!("\n-# {votes}"));
        }
        if track.license != "CC0-1.0" {
            status.push_str(&format!(
                "\n-# [Credits & licence]({})",
                track.credits_url()
            ));
        }
        components.push(serde_json::json!({"type":10,"content":status}));
    }
    if !stopped && session.tips.load(Ordering::SeqCst) {
        components.push(serde_json::json!({"type":10,"content":"-# Like and Dislike personalize your station using Music's approved catalog. Confirmations are private. /library shows or removes your ratings."}));
    }
    // Keep controls outside the container's side padding in narrow voice chats.
    let mut layout = vec![serde_json::json!({"type":17,"components":components})];
    layout.extend(
        controls(session, generation, stopped)?
            .into_iter()
            .map(serde_json::to_value)
            .collect::<Result<Vec<_>, _>>()?,
    );
    // Serenity's typed builders predate V2; its HTTP client still handles uploads and rate limits.
    Ok(serde_json::json!({"flags":1 << 15,"components":layout,"allowed_mentions":{"parse":[]}}))
}

pub async fn message(
    catalog: &Catalog,
    session: &Session,
) -> Result<(serde_json::Value, Vec<CreateAttachment>), Error> {
    let mut body = payload(catalog, session, false, None)?;
    let mut files = Vec::new();
    let track = &catalog.stations[session.station].tracks[session.current.load(Ordering::SeqCst)];
    if track.cover.is_some() {
        files.push(super::player_artwork::attachment(track).await?);
        body["attachments"] = serde_json::to_value(&files)?;
    }
    Ok((body, files))
}

pub async fn post(
    ctx: &Context,
    catalog: &Catalog,
    session: &Session,
) -> Result<Arc<super::PlayerPanel>, Error> {
    let runtime = session
        .runtime
        .as_ref()
        .ok_or("Voice session unavailable. Use /play again.")?;
    let channel = runtime.channel;
    {
        let guild = ctx
            .cache
            .guild(runtime.guild)
            .ok_or("The server is still loading.")?;
        let voice = guild
            .channels
            .get(&channel)
            .ok_or("Voice channel unavailable.")?;
        #[allow(deprecated)]
        let permissions = voice.permissions_for_user(&ctx.cache, ctx.cache.current_user().id)?;
        let mut required =
            Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES | Permissions::EMBED_LINKS;
        if catalog.stations[session.station].tracks[session.current.load(Ordering::SeqCst)]
            .cover
            .is_some()
        {
            required |= Permissions::ATTACH_FILES;
        }
        if !permissions.contains(required) {
            return Err("Check Music's View Channel, Send Messages, Embed Links and Attach Files permissions in the voice channel.".into());
        }
    }
    remove(&ctx.http, session).await?;
    let (body, files) = message(catalog, session).await?;
    let message = ctx.http.send_message(channel, files, &body).await.map_err(|error| {
        eprintln!("Could not post the voice-channel player: {error}");
        "Could not post the player in voice chat. Check Music's channel permissions and try /panel."
    })?;
    Ok(Arc::new(super::PlayerPanel {
        channel,
        message: message.id,
        update: tokio::sync::Mutex::new(attached_cover(
            catalog.stations[session.station].tracks[session.current.load(Ordering::SeqCst)]
                .cover
                .as_ref(),
            &message,
        )),
    }))
}

pub async fn remove(http: &Http, session: &Session) -> Result<(), Error> {
    let panel = session
        .panel
        .lock()
        .map_err(|_| "Panel unavailable.")?
        .clone();
    let Some(panel) = panel else {
        return Ok(());
    };
    let _guard = panel.update.lock().await;
    if !session
        .panel
        .lock()
        .map_err(|_| "Panel unavailable.")?
        .as_ref()
        .is_some_and(|p| Arc::ptr_eq(p, &panel))
    {
        return Ok(());
    }
    if let Err(error) = panel.channel.delete_message(http, panel.message).await {
        let missing = matches!(&error, serenity::Error::Http(serenity::http::HttpError::UnsuccessfulRequest(response)) if response.status_code.as_u16() == 404);
        if !missing {
            return Err(error.into());
        }
    }
    let mut current = session.panel.lock().map_err(|_| "Panel unavailable.")?;
    if current.as_ref().is_some_and(|p| Arc::ptr_eq(p, &panel)) {
        *current = None;
    }
    Ok(())
}

pub async fn refresh(http: &Http, catalog: &Catalog, session: &Arc<Session>, stopped: bool) {
    if stopped {
        if let Err(error) = remove(http, session).await {
            eprintln!("Could not delete the station player: {error}");
        }
        return;
    }
    let panel = session.panel.lock().ok().and_then(|p| p.clone());
    let Some(panel) = panel else {
        return;
    };
    // Shared across manual skips, so an old HTTP edit completes before the new player's edit.
    let mut panel_guard = panel.update.lock().await;
    if !session
        .panel
        .lock()
        .ok()
        .is_some_and(|p| p.as_ref().is_some_and(|p| Arc::ptr_eq(p, &panel)))
    {
        return;
    }
    let (channel, message) = (panel.channel, panel.message);
    if !stopped && !session.active.load(Ordering::SeqCst) {
        return;
    }
    let generation = session.generation.load(Ordering::SeqCst);
    let file = catalog.stations[session.station].tracks[session.current.load(Ordering::SeqCst)]
        .cover
        .as_ref();
    let update = async {
        let existing = panel_guard.as_ref().filter(|a| Some(&a.file) == file);
        let mut body = payload(catalog, session, stopped, existing.map(|a| a.url.as_str()))?;
        body["content"] = serde_json::Value::Null;
        body["embeds"] = serde_json::json!([]);
        let mut files = Vec::new();
        if existing.is_none() {
            if file.is_some() {
                let track = &catalog.stations[session.station].tracks
                    [session.current.load(Ordering::SeqCst)];
                files.push(super::player_artwork::attachment(track).await?);
            }
            body["attachments"] = serde_json::to_value(&files)?;
        }
        Ok::<_, Error>((body, files))
    }
    .await;
    if !stopped && generation != session.generation.load(Ordering::SeqCst) {
        return;
    }
    match update {
        Ok((body, files)) => match http.edit_message(channel, message, &body, files).await {
            Ok(message) => *panel_guard = attached_cover(file, &message),
            Err(error) => eprintln!("Could not update the station player message: {error}"),
        },
        Err(error) => eprintln!("Could not prepare the station player: {error}"),
    }
    drop(panel_guard);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v2_player_messages_and_button_events_keep_their_control_identity() {
        let catalog: Catalog = serde_json::from_str(include_str!("../catalog.json")).unwrap();
        let session = Session::new(0, 0, 123, Default::default(), None);
        let body = payload(&catalog, &session, false, None).unwrap();
        let message = serde_json::json!({"id":"456","channel_id":"123","author":{"id":"789","username":"Music","discriminator":"0","avatar":null},"content":"","timestamp":"2026-11-01T20:00:00Z","edited_timestamp":null,"tts":false,"mention_everyone":false,"mentions":[],"mention_roles":[],"pinned":false,"type":0,"embeds":[],"attachments":[],"components":body["components"],"flags":32768});
        let button: ComponentInteraction = serde_json::from_value(serde_json::json!({"id":"111","application_id":"222","channel_id":"123","token":"synthetic-interaction","version":1,"locale":"en-US","entitlements":[],"attachment_size_limit":8388608,"user":{"id":"789","username":"Test","discriminator":"0","avatar":null},"data":{"custom_id":"vozen:action:pause:123:1","component_type":2},"message":message})).unwrap();
        assert_eq!(button.message.components.len(), 2);
        assert_eq!(button.message.id, MessageId::new(456));
        assert_eq!(button.data.custom_id, "vozen:action:pause:123:1");
        let mut data = serde_json::to_value(&button).unwrap();
        data["data"] = serde_json::json!({"id":"1","name":"pause","type":1,"options":[]});
        let command: CommandInteraction = serde_json::from_value(data).unwrap();
        assert_eq!(command.user.id, button.user.id);
        assert_eq!(command.channel_id, button.channel_id);
        assert_eq!(command.data.name, "pause");
    }

    #[tokio::test]
    async fn player_buttons_keep_playback_silent_and_like_confirmations_private() {
        use axum::{
            Router,
            body::{Body, to_bytes},
            http::{Request, StatusCode},
            response::IntoResponse,
        };
        let message = serde_json::json!({"id":"456","channel_id":"123","author":{"id":"789","username":"Music","discriminator":"0","avatar":null},"content":"","timestamp":"2026-11-01T20:00:00Z","edited_timestamp":null,"tts":false,"mention_everyone":false,"mentions":[],"mention_roles":[],"pinned":false,"type":0,"embeds":[],"attachments":[]});
        let mut button: ComponentInteraction = serde_json::from_value(serde_json::json!({"id":"111","application_id":"222","channel_id":"123","token":"synthetic-interaction","version":1,"locale":"en-US","entitlements":[],"attachment_size_limit":8388608,"user":{"id":"789","username":"Test","discriminator":"0","avatar":null},"data":{"custom_id":"vozen:skip:1:1","component_type":2},"message":message})).unwrap();
        let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured = requests.clone();
        let app = Router::new().fallback(move |request: Request<Body>| {
            let captured = captured.clone();
            let message = message.clone();
            async move {
                let path = request.uri().path().to_owned();
                let body = to_bytes(request.into_body(), 1024 * 1024).await.unwrap();
                let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
                captured.lock().unwrap().push((path.clone(), body));
                if path.contains("/interactions/") {
                    StatusCode::NO_CONTENT.into_response()
                } else {
                    axum::Json(message).into_response()
                }
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let http = serenity::http::HttpBuilder::new("synthetic-token")
            .application_id(ApplicationId::new(222))
            .proxy(format!("http://{address}"))
            .ratelimiter_disabled(true)
            .build();
        acknowledge_button(&http, &button).await.unwrap();
        for result in [
            "Track skipped.",
            "Paused.",
            "Resumed.",
            "Liked.",
            "Disliked.",
            "Stopped.",
            "Vote recorded. Votes: 1/3.",
        ] {
            finish_button(&http, &button, Ok(result.into()))
                .await
                .unwrap();
        }
        {
            let records = requests.lock().unwrap();
            assert_eq!(
                records[0].1["type"], 6,
                "A button must acknowledge without creating a reply"
            );
            assert_eq!(
                records.len(),
                1,
                "Successful buttons must not add chat messages"
            );
            assert!(records[0].1["data"].is_null());
        }
        button.data.custom_id = "vozen:action:like:1:1".into();
        for text in [
            "Added this recording to your likes.",
            "Removed this recording from your likes.",
        ] {
            finish_button(&http, &button, Ok(text.into()))
                .await
                .unwrap();
        }
        button.data.custom_id = "vozen:action:dislike:1:1".into();
        finish_button(
            &http,
            &button,
            Ok("Your rating was saved for this server.".into()),
        )
        .await
        .unwrap();
        finish_button(&http, &button, Err("Join the same voice channel.".into()))
            .await
            .unwrap();
        let records = requests.lock().unwrap();
        assert_eq!(records.len(), 5);
        assert_eq!(
            records[1].1["content"],
            "Added this recording to your likes."
        );
        assert_eq!(
            records[2].1["content"],
            "Removed this recording from your likes."
        );
        assert_eq!(
            records[3].1["content"],
            "Your rating was saved for this server."
        );
        assert_eq!(records[4].1["content"], "Join the same voice channel.");
        for (path, body) in &records[1..] {
            assert!(path.contains("/webhooks/222/"));
            assert_eq!(body["flags"], 64);
            assert_eq!(body["allowed_mentions"]["parse"], serde_json::json!([]));
        }
        server.abort();
    }

    #[tokio::test]
    async fn track_changes_upload_native_covers_and_same_track_edits_keep_them() {
        use axum::{
            Router,
            body::{Body, to_bytes},
            http::Request,
        };
        let catalog =
            Arc::new(Catalog::load(std::path::Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap());
        let names: Vec<String> = catalog.stations[0].tracks[..2]
            .iter()
            .map(|t| super::super::player_artwork::filename(t.cover.as_ref().unwrap()))
            .collect();
        let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorded = requests.clone();
        let response_names = names.clone();
        let app = Router::new().fallback(move |request: Request<Body>| {
            let recorded = recorded.clone();
            let names = response_names.clone();
            async move {
                assert_eq!(request.method(), "PATCH");
                let content_type = request.headers()["content-type"].to_str().unwrap().to_owned();
                let body = to_bytes(request.into_body(), 8 * 1024 * 1024).await.unwrap();
                let mut records = recorded.lock().unwrap();
                let index = records.len();
                records.push((content_type, body));
                axum::Json(serde_json::json!({
                    "id":"456", "channel_id":"123", "author":{"id":"789","username":"Music","discriminator":"0","avatar":null},
                    "content":"", "timestamp":"2026-11-01T20:00:00Z", "edited_timestamp":null, "tts":false,
                    "mention_everyone":false,"mentions":[],"mention_roles":[],"pinned":false,"type":0,"embeds":[],"flags":32768,
                    "components":[{"type":12,"items":[{"media":{"url":format!("https://cdn.discordapp.com/attachments/123/17/{}",names[if index==2 {1} else {0}])}}]},{"type":10,"content":"Track title"}],
                    "attachments":[{"id":"17","filename":names[if index==2 {1} else {0}],"size":1024,"url":format!("https://cdn.discordapp.com/attachments/123/17/{}",names[if index==2 {1} else {0}]),"proxy_url":format!("https://media.discordapp.net/attachments/123/17/{}",names[if index==2 {1} else {0}]),"width":1254,"height":1254}]
                }))
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let http = serenity::http::HttpBuilder::new("synthetic-token")
            .proxy(format!("http://{addr}"))
            .ratelimiter_disabled(true)
            .build();
        let session = Arc::new(Session::new(
            0,
            0,
            1,
            Default::default(),
            Some(Arc::new(super::super::PlayerPanel {
                channel: ChannelId::new(123),
                message: MessageId::new(456),
                update: tokio::sync::Mutex::new(None),
            })),
        ));
        refresh(&http, &catalog, &session, false).await;
        session.paused.store(true, Ordering::SeqCst);
        refresh(&http, &catalog, &session, false).await;
        session.current.store(1, Ordering::SeqCst);
        session.generation.store(2, Ordering::SeqCst);
        refresh(&http, &catalog, &session, false).await;
        session.current.store(0, Ordering::SeqCst);
        session.generation.store(3, Ordering::SeqCst);
        refresh(&http, &catalog, &session, false).await;
        let records = requests.lock().unwrap();
        assert_eq!(records.len(), 4);
        for (index, name) in [(0, &names[0]), (2, &names[1]), (3, &names[0])] {
            assert!(
                records[index].0.starts_with("multipart/form-data"),
                "Track change must upload artwork, got {}",
                records[index].0
            );
            let body = String::from_utf8_lossy(&records[index].1);
            assert!(body.contains(&format!("attachment://{name}")));
            assert!(body.contains(&format!("filename=\"{name}\"")));
            assert!(!body.contains("https://api.vozen.org/music/covers/"));
        }
        assert!(records[1].0.starts_with("application/json"));
        let same: serde_json::Value = serde_json::from_slice(&records[1].1).unwrap();
        assert!(same.get("attachments").is_none());
        assert_eq!(
            same["components"][0]["components"][0]["items"][0]["media"]["url"],
            format!("https://cdn.discordapp.com/attachments/123/17/{}", names[0])
        );
        server.abort();
    }

    #[tokio::test]
    async fn stopped_players_are_deleted_and_missing_messages_are_harmless() {
        use axum::{
            Router,
            body::Body,
            http::{Request, StatusCode},
            response::IntoResponse,
        };
        use std::sync::atomic::AtomicUsize;
        for status in [204, 404, 403] {
            let requests = Arc::new(AtomicUsize::new(0));
            let count = requests.clone();
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let app = Router::new().fallback(move |request: Request<Body>| {
                let count = count.clone();
                async move {
                    assert_eq!(request.method(), "DELETE");
                    assert!(request.uri().path().ends_with("/channels/123/messages/456"));
                    count.fetch_add(1, Ordering::SeqCst);
                    (
                        StatusCode::from_u16(status).unwrap(),
                        axum::Json(serde_json::json!({"code":10008,"message":"Unknown Message"})),
                    )
                        .into_response()
                }
            });
            let server = tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });
            let http = Arc::new(
                serenity::http::HttpBuilder::new("synthetic-token")
                    .proxy(format!("http://{addr}"))
                    .ratelimiter_disabled(true)
                    .build(),
            );
            let session = Arc::new(Session::new(
                0,
                0,
                1,
                Default::default(),
                Some(Arc::new(super::super::PlayerPanel {
                    channel: ChannelId::new(123),
                    message: MessageId::new(456),
                    update: tokio::sync::Mutex::new(None),
                })),
            ));
            let panel = session.panel.lock().unwrap().clone().unwrap();
            let gate = panel.update.lock().await;
            let removal = tokio::spawn({
                let http = http.clone();
                let session = session.clone();
                async move { remove(&http, &session).await }
            });
            tokio::task::yield_now().await;
            let update = (status != 403).then(|| {
                tokio::spawn({
                    let http = http.clone();
                    let session = session.clone();
                    async move {
                        refresh(&http, &Catalog { stations: vec![] }, &session, false).await;
                    }
                })
            });
            tokio::task::yield_now().await;
            assert_eq!(requests.load(Ordering::SeqCst), 0);
            drop(gate);
            let result = removal.await.unwrap();
            if let Some(update) = update {
                update.await.unwrap();
            }
            assert_eq!(
                result.is_ok(),
                status != 403,
                "Delete status {status}: {result:?}"
            );
            assert_eq!(session.panel.lock().unwrap().is_none(), status != 403);
            assert_eq!(requests.load(Ordering::SeqCst), 1);
            if status != 403 {
                refresh(&http, &Catalog { stations: vec![] }, &session, true).await;
                assert_eq!(requests.load(Ordering::SeqCst), 1);
            }
            server.abort();
        }
    }
    #[test]
    fn rejects_invalid_or_zero_player_targets() {
        assert_eq!(target("vozen:skip:123:2").unwrap(), (123, 2, false));
        assert_eq!(target("vozen:previous:123:2").unwrap(), (123, 2, true));
        for id in [
            "vozen:previous:0:1",
            "vozen:previous:1:0",
            "skip:123:2",
            "vozen:skip:0:1",
            "vozen:skip:1:0",
            "vozen:skip:1:1:extra",
            "vozen:skip:no:1",
        ] {
            assert!(target(id).is_err());
        }
    }

    #[test]
    fn shared_and_private_controls_bind_to_one_track_and_disable_on_stop() {
        let mut policy = vozen_music::skip::SkipSettings::default();
        let session = Session::new(0, 0, 123, policy.clone(), None);
        let buttons = serde_json::to_value(controls(&session, 2, false).unwrap()).unwrap();
        assert_eq!(buttons[0]["components"][3]["custom_id"], "vozen:skip:123:2");
        assert_eq!(
            buttons[0]["components"][1]["custom_id"],
            "vozen:previous:123:2"
        );
        assert_eq!(buttons[0]["components"][1]["disabled"], true);
        assert_eq!(
            buttons[0]["components"][2]["custom_id"],
            "vozen:action:pause:123:2"
        );
        assert_eq!(
            buttons[0]["components"][0]["custom_id"],
            "vozen:action:volume-down:123:2"
        );
        assert_eq!(
            buttons[0]["components"][4]["custom_id"],
            "vozen:action:volume-up:123:2"
        );
        assert_eq!(buttons[0]["components"][0]["disabled"], false);
        assert_eq!(buttons[0]["components"][4]["disabled"], true);
        assert_eq!(buttons[1]["components"][0]["emoji"]["name"], "❤️");
        assert!(buttons[1]["components"][0].get("label").is_none());
        assert_eq!(buttons[1]["components"][1]["emoji"]["name"], "💔");
        assert_eq!(
            buttons[1]["components"][1]["custom_id"],
            "vozen:action:dislike:123:2"
        );
        assert_eq!(buttons[1]["components"][2]["label"], "↻");
        assert_eq!(buttons[1]["components"][3]["label"], "Leave");
        assert_eq!(buttons[1]["components"][3]["style"], 4);
        assert_eq!(buttons.as_array().unwrap().len(), 2);
        assert_eq!(buttons[0]["components"].as_array().unwrap().len(), 5);
        for (button, icon) in buttons[0]["components"]
            .as_array()
            .unwrap()
            .iter()
            .zip(["🔉", "⏮", "⏸", "⏭", "🔊"])
        {
            assert_eq!(button["emoji"]["name"], icon);
            assert!(
                button.get("label").is_none(),
                "Transport buttons stay icon-only"
            );
        }
        assert_eq!(buttons[1]["components"].as_array().unwrap().len(), 4);
        assert_eq!(buttons[0]["components"][2]["style"], 3);
        assert_eq!(buttons[1]["components"][2]["style"], 2);
        session.looping.store(true, Ordering::SeqCst);
        session.set_track(1).unwrap();
        assert_eq!(
            buttons[1]["components"][0]["custom_id"],
            "vozen:action:like:123:2"
        );
        policy.mode = SkipMode::Majority;
        *session.policy.lock().unwrap() = policy;
        let buttons = serde_json::to_value(controls(&session, 3, false).unwrap()).unwrap();
        assert_eq!(buttons[0]["components"][1]["disabled"], false);
        assert_eq!(buttons[1]["components"][2]["style"], 3);
        assert_eq!(buttons[1]["components"][0]["style"], 2);
        session.volume.store(0, Ordering::SeqCst);
        let buttons = serde_json::to_value(controls(&session, 3, false).unwrap()).unwrap();
        assert_eq!(buttons[0]["components"][0]["disabled"], true);
        assert_eq!(buttons[0]["components"][4]["disabled"], false);
        session.volume.store(50, Ordering::SeqCst);
        session.paused.store(true, Ordering::SeqCst);
        let buttons = serde_json::to_value(controls(&session, 3, false).unwrap()).unwrap();
        assert_eq!(buttons[0]["components"][0]["disabled"], false);
        assert_eq!(buttons[0]["components"][4]["disabled"], false);
        assert_eq!(
            buttons[0]["components"][2]["custom_id"],
            "vozen:action:resume:123:3"
        );
        {
            let mut votes = session.votes.lock().unwrap();
            votes.voters.insert(456);
            votes.required = 3;
        }
        let buttons = serde_json::to_value(controls(&session, 3, false).unwrap()).unwrap();
        assert_eq!(buttons[0]["components"][2]["emoji"]["name"], "▶");
        assert!(
            buttons[0]["components"]
                .as_array()
                .unwrap()
                .iter()
                .all(|b| b.get("label").is_none())
        );
        assert_eq!(vote_progress(&session).unwrap(), "Next: 1/3 votes");
        {
            let mut votes = session.previous_votes.lock().unwrap();
            votes.voters.extend([789, 987]);
            votes.required = 4;
        }
        assert_eq!(
            vote_progress(&session).unwrap(),
            "Previous: 2/4 votes · Next: 1/3 votes"
        );
        let catalog: Catalog = serde_json::from_str(include_str!("../catalog.json")).unwrap();
        let card = payload(&catalog, &session, false, None).unwrap();
        assert!(
            card["components"][0]["components"][1]["content"]
                .as_str()
                .unwrap()
                .contains("Previous: 2/4 votes · Next: 1/3 votes")
        );
        session.policy.lock().unwrap().mode = SkipMode::Instant;
        assert!(vote_progress(&session).unwrap().is_empty());
        let disabled = serde_json::to_value(controls(&session, 3, true).unwrap()).unwrap();
        for row in disabled.as_array().unwrap() {
            for button in row["components"].as_array().unwrap() {
                assert_eq!(button["disabled"], true);
            }
        }
    }

    #[test]
    fn volume_buttons_step_ten_points_and_clamp_at_both_limits() {
        for (level, down, up) in [
            (0, 0, 10),
            (5, 0, 15),
            (50, 40, 60),
            (95, 85, 100),
            (100, 90, 100),
        ] {
            assert_eq!(button_volume("volume-down", level), Some(down));
            assert_eq!(button_volume("volume-up", level), Some(up));
        }
        assert_eq!(button_volume("volume", 50), None);
        assert_eq!(button_volume("unknown", 50), None);
    }

    #[tokio::test]
    async fn by_recordings_link_to_public_credits_without_adding_controls() {
        let mut catalog: Catalog = serde_json::from_str(include_str!("../catalog.json")).unwrap();
        catalog.stations[0].tracks[0].license = "CC-BY-4.0".into();
        let session = Session::new(0, 0, 123, vozen_music::skip::SkipSettings::default(), None);
        let card = payload(&catalog, &session, false, None).unwrap();
        assert!(
            card["components"][0]["components"][1]["content"]
                .as_str()
                .unwrap()
                .contains(&catalog.stations[0].tracks[0].credits_url())
        );
        assert_eq!(
            card["components"][1]["components"]
                .as_array()
                .unwrap()
                .len(),
            5
        );
        assert_eq!(
            card["components"][2]["components"]
                .as_array()
                .unwrap()
                .len(),
            4
        );
    }

    #[tokio::test]
    async fn player_card_keeps_accessible_metadata_and_playback_controls_without_a_credits_footer()
    {
        let catalog: Catalog = serde_json::from_str(include_str!("../catalog.json")).unwrap();
        let mut policy = vozen_music::skip::SkipSettings::default();
        let session = Session::new(0, 0, 123, policy.clone(), None);
        let card = payload(&catalog, &session, false, None).unwrap();
        let track = &catalog.stations[0].tracks[0];
        assert_eq!(card["flags"], 32768);
        assert!(card.get("embeds").is_none());
        assert!(card.get("content").is_none());
        assert_eq!(card["allowed_mentions"]["parse"], serde_json::json!([]));
        assert_eq!(card["components"][0]["type"], 17);
        let components = card["components"][0]["components"].as_array().unwrap();
        assert_eq!(components[0]["type"], 12);
        assert_eq!(components[1]["type"], 10);
        let text = components[1]["content"].as_str().unwrap();
        assert_eq!(
            components[0]["items"][0]["description"],
            format!("{} by {}", track.title, track.artist)
        );
        assert!(!text.contains(&track.title));
        assert!(!text.contains(&track.artist));
        assert!(text.contains("Playing · 100%"));
        assert_eq!(components.len(), 2);
        assert_eq!(card["components"].as_array().unwrap().len(), 3);
        assert_eq!(card["components"][1]["type"], 1);
        assert_eq!(card["components"][2]["type"], 1);
        assert_eq!(
            card["components"][1]["components"]
                .as_array()
                .unwrap()
                .len(),
            5
        );
        assert!(!card.to_string().contains(&track.source_url));
        assert!(!card.to_string().contains("Credits & licence"));
        assert!(!card.to_string().contains(&track.license));
        assert!(!text.contains("Controls are for"));
        session.paused.store(true, Ordering::SeqCst);
        session.volume.store(40, Ordering::SeqCst);
        policy.access = vozen_music::skip::SkipAccess::Moderators;
        policy.mode = SkipMode::Majority;
        policy.majority_scope = vozen_music::skip::MajorityScope::Eligible;
        *session.policy.lock().unwrap() = policy;
        let paused = payload(&catalog, &session, false, None).unwrap();
        assert!(
            paused["components"][0]["components"][1]["content"]
                .as_str()
                .unwrap()
                .contains("Paused · 40%")
        );
        assert_eq!(
            paused["components"][1]["components"][2]["custom_id"],
            "vozen:action:resume:123:1"
        );
        let stopped = payload(&catalog, &session, true, None).unwrap();
        assert_eq!(
            stopped["components"][0]["components"][0]["content"],
            "Playback stopped. Use /play to start a station."
        );
        let mut legacy = catalog.clone();
        legacy.stations[0].tracks[0].cover = None;
        let legacy = payload(&legacy, &session, false, None).unwrap();
        assert_eq!(legacy["components"][0]["components"][0]["type"], 10);
        assert!(
            legacy["components"][0]["components"][0]["content"]
                .as_str()
                .unwrap()
                .contains(&track.title)
        );
    }

    #[test]
    fn player_selects_the_matching_cover_for_every_recording() {
        let mut catalog: Catalog = serde_json::from_str(include_str!("../catalog.json")).unwrap();
        catalog.stations.push(vozen_music::Station {
            id: "my".into(),
            name: "My Station".into(),
            tracks: catalog
                .stations
                .iter()
                .flat_map(|s| s.tracks.clone())
                .collect(),
        });
        for (station_index, station) in catalog.stations.iter().enumerate() {
            let session = Session::new(station_index, 0, 1, Default::default(), None);
            for (index, track) in station.tracks.iter().enumerate() {
                session.current.store(index, Ordering::SeqCst);
                let card = payload(&catalog, &session, false, None).unwrap();
                let name = super::super::player_artwork::filename(track.cover.as_ref().unwrap());
                assert_eq!(
                    card["components"][0]["components"][0]["items"][0]["description"],
                    format!("{} by {}", track.title, track.artist)
                );
                assert_eq!(
                    card["components"][0]["components"][0]["items"][0]["media"]["url"],
                    format!("attachment://{name}")
                );
            }
            let stopped = payload(&catalog, &session, true, None).unwrap();
            assert!(
                !stopped["components"][0]["components"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|c| c["type"] == 12)
            );
        }
    }
}
