use super::{Catalog, Error, Session};
use serenity::all::*;
use std::sync::{Arc, atomic::Ordering};
use vozen_music::skip::{MajorityScope, SkipAccess, SkipMode};

pub fn target(id: &str) -> Result<(u64, usize), Error> {
    let parts: Vec<_> = id.split(':').collect();
    if parts.len() != 4 || parts[0] != "vozen" || parts[1] != "skip" {
        return Err("Invalid player control.".into());
    }
    let session = parts[2].parse::<u64>()?;
    let generation = parts[3].parse::<usize>()?;
    if session == 0 || generation == 0 {
        return Err("Invalid player control.".into());
    }
    Ok((session, generation))
}

pub fn controls(
    session: &Session,
    generation: usize,
    stopped: bool,
) -> Result<Vec<CreateActionRow>, Error> {
    let policy = session
        .policy
        .lock()
        .map_err(|_| "Player settings are unavailable.")?;
    let votes = session
        .votes
        .lock()
        .map_err(|_| "Skip votes are unavailable.")?;
    let label = if policy.mode == SkipMode::Instant {
        "Skip".to_owned()
    } else if votes.voters.is_empty() {
        "Vote to skip".to_owned()
    } else {
        format!("Skip ({}/{})", votes.voters.len(), votes.required)
    };
    let action = |name: &str, label: &str| {
        CreateButton::new(format!(
            "vozen:action:{name}:{}:{generation}",
            session.control_id
        ))
        .label(label)
        .style(if name == "stop" {
            ButtonStyle::Danger
        } else if matches!(name, "pause" | "resume") {
            ButtonStyle::Primary
        } else {
            ButtonStyle::Secondary
        })
        .disabled(stopped)
    };
    Ok(vec![
        CreateActionRow::Buttons(vec![
            action(
                if session.paused.load(Ordering::SeqCst) {
                    "resume"
                } else {
                    "pause"
                },
                if session.paused.load(Ordering::SeqCst) {
                    "Resume"
                } else {
                    "Pause"
                },
            ),
            CreateButton::new(format!("vozen:skip:{}:{generation}", session.control_id))
                .label(label)
                .style(ButtonStyle::Secondary)
                .disabled(stopped),
            action("stop", "Stop"),
        ]),
        CreateActionRow::Buttons(vec![action("like", "Like"), action("dislike", "Dislike")]),
    ])
}

fn embed(catalog: &Catalog, session: &Session, stopped: bool) -> Result<CreateEmbed, Error> {
    let station = &catalog.stations[session.station];
    let header = CreateEmbed::new().author(CreateEmbedAuthor::new(&station.name));
    if stopped {
        return Ok(header
            .color(0x677286)
            .title("Playback stopped")
            .description("Use /play to start a station."));
    }
    let track = &station.tracks[session.current.load(Ordering::SeqCst)];
    let policy = session
        .policy
        .lock()
        .map_err(|_| "Player settings are unavailable.")?;
    let access = match policy.access {
        SkipAccess::Everyone => "Everyone in the call",
        SkipAccess::Moderators => "Moderators and server managers",
    };
    let mode = match (policy.mode, policy.majority_scope) {
        (SkipMode::Instant, _) => "One click",
        (SkipMode::Majority, MajorityScope::Call) => "Majority of the call",
        (SkipMode::Majority, MajorityScope::Eligible) => "Majority of eligible members",
    };
    let paused = session.paused.load(Ordering::SeqCst);
    let footer = if session.tips.load(Ordering::SeqCst) {
        "Controls are for this call · Like and Dislike shape future tracks · /library shows your ratings"
    } else {
        "Controls are for this call · /skip-settings shows the full skip rules"
    };
    Ok(header
        .color(if paused { 0x9B8AFB } else { 0x59D6CB })
        .title(&track.title)
        .description(format!("by {}", track.artist))
        .field(
            "Playback",
            format!(
                "{} · Volume {}%",
                if paused { "Paused" } else { "Playing" },
                session.volume.load(Ordering::SeqCst)
            ),
            true,
        )
        .field("Skip", format!("{access}\n{mode}"), true)
        .field(
            "Credits",
            format!(
                "[Source & licence]({}) · {}",
                track.source_url, track.license
            ),
            false,
        )
        .footer(CreateEmbedFooter::new(footer)))
}

pub fn message(catalog: &Catalog, session: &Session) -> Result<CreateMessage, Error> {
    Ok(CreateMessage::new()
        .embed(embed(catalog, session, false)?)
        .components(controls(
            session,
            session.generation.load(Ordering::SeqCst).max(1),
            false,
        )?)
        .allowed_mentions(CreateAllowedMentions::new()))
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
        if !permissions.contains(
            Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES | Permissions::EMBED_LINKS,
        ) {
            return Err("Music needs View Channel, Send Messages and Embed Links in the voice channel to post the player.".into());
        }
    }
    let message = channel.send_message(&ctx.http, message(catalog, session)?).await.map_err(|error| {
        eprintln!("Could not post the voice-channel player: {error}");
        "Could not post the player in voice chat. Check Music's channel permissions and try /panel."
    })?;
    Ok(Arc::new(super::PlayerPanel {
        channel,
        message: message.id,
        update: tokio::sync::Mutex::new(()),
    }))
}

pub async fn refresh(http: &Http, catalog: &Catalog, session: &Arc<Session>, stopped: bool) {
    let panel = session.panel.lock().ok().and_then(|p| p.clone());
    let Some(panel) = panel else {
        return;
    };
    // Shared across manual skips, so an old HTTP edit completes before the new player's edit.
    let panel_guard = panel.update.lock().await;
    let (channel, message) = (panel.channel, panel.message);
    if !stopped && !session.active.load(Ordering::SeqCst) {
        return;
    }
    let generation = session.generation.load(Ordering::SeqCst);
    let update = (|| -> Result<EditMessage, Error> {
        Ok(EditMessage::new()
            .content("")
            .embeds(vec![embed(catalog, session, stopped)?])
            .components(controls(session, generation, stopped)?)
            .allowed_mentions(CreateAllowedMentions::new()))
    })();
    if !stopped && generation != session.generation.load(Ordering::SeqCst) {
        return;
    }
    match update {
        Ok(update) => {
            if channel.edit_message(http, message, update).await.is_err() {
                eprintln!("Could not update the station player message.");
            }
        }
        Err(_) => eprintln!("Could not read station player state."),
    }
    drop(panel_guard);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_invalid_or_zero_player_targets() {
        assert_eq!(target("vozen:skip:123:2").unwrap(), (123, 2));
        for id in [
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
        assert_eq!(buttons[0]["components"][1]["custom_id"], "vozen:skip:123:2");
        assert_eq!(buttons[0]["components"][1]["label"], "Skip");
        assert_eq!(buttons[0]["components"].as_array().unwrap().len(), 3);
        assert_eq!(buttons[1]["components"].as_array().unwrap().len(), 2);
        assert_eq!(buttons[0]["components"][2]["style"], 4);
        assert_eq!(
            buttons[1]["components"][0]["custom_id"],
            "vozen:action:like:123:2"
        );
        policy.mode = SkipMode::Majority;
        *session.policy.lock().unwrap() = policy;
        let buttons = serde_json::to_value(controls(&session, 3, false).unwrap()).unwrap();
        assert_eq!(buttons[0]["components"][1]["label"], "Vote to skip");
        let disabled = serde_json::to_value(controls(&session, 3, true).unwrap()).unwrap();
        for row in disabled.as_array().unwrap() {
            for button in row["components"].as_array().unwrap() {
                assert_eq!(button["disabled"], true);
            }
        }
    }

    #[test]
    fn player_card_keeps_credits_and_tracks_playback_and_voting_states() {
        let catalog: Catalog = serde_json::from_str(include_str!("../catalog.json")).unwrap();
        let mut policy = vozen_music::skip::SkipSettings::default();
        let session = Session::new(0, 0, 123, policy.clone(), None);
        let card = serde_json::to_value(message(&catalog, &session).unwrap()).unwrap();
        let track = &catalog.stations[0].tracks[0];
        assert_eq!(card["embeds"][0]["title"], track.title);
        assert_eq!(
            card["embeds"][0]["description"],
            format!("by {}", track.artist)
        );
        assert!(
            card["embeds"][0]["fields"][2]["value"]
                .as_str()
                .unwrap()
                .contains(&track.source_url)
        );
        assert!(
            card["embeds"][0]["fields"][2]["value"]
                .as_str()
                .unwrap()
                .contains(&track.license)
        );
        assert_eq!(
            card["embeds"][0]["fields"][1]["value"],
            "Everyone in the call\nOne click"
        );
        session.paused.store(true, Ordering::SeqCst);
        session.volume.store(40, Ordering::SeqCst);
        policy.access = SkipAccess::Moderators;
        policy.mode = SkipMode::Majority;
        policy.majority_scope = MajorityScope::Eligible;
        *session.policy.lock().unwrap() = policy;
        let paused = serde_json::to_value(embed(&catalog, &session, false).unwrap()).unwrap();
        assert_eq!(paused["fields"][0]["value"], "Paused · Volume 40%");
        assert_eq!(
            paused["fields"][1]["value"],
            "Moderators and server managers\nMajority of eligible members"
        );
        let stopped = serde_json::to_value(embed(&catalog, &session, true).unwrap()).unwrap();
        assert_eq!(stopped["title"], "Playback stopped");
        assert!(stopped.get("fields").is_none());
    }
}
