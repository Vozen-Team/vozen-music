use vozen_music::expansion::{Privacy, RadioMode, ServerSettings};
static PROFILE_UPLOADS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);

async fn profile_upload_guard(
    State(state): State<Arc<Dashboard>>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    let _permit = if request.method() == axum::http::Method::PUT {
        if let Err(error) = state.login(request.headers(), true).await {
            return error.into_response();
        }
        match PROFILE_UPLOADS.try_acquire() {
            Ok(permit) => Some(permit),
            Err(_) => {
                return WebError(
                    StatusCode::TOO_MANY_REQUESTS,
                    "Profile uploads are busy. Try again shortly.",
                )
                .into_response();
            }
        }
    } else {
        None
    };
    next.run(request).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TimerEdit {
    minutes: Option<i64>,
    cancel: bool,
}
async fn playback_timer(
    State(state): State<Arc<Dashboard>>,
    headers: HeaderMap,
    RoutePath(value): RoutePath<String>,
    Json(edit): Json<TimerEdit>,
) -> WebResult<Json<serde_json::Value>> {
    let login = state.login(&headers, true).await?;
    let guild = GuildId::new(parse_id(&value)?);
    let session = state.bot.sessions.lock().await.get(&guild).cloned();
    state.manager(guild, &login).await?;
    let session = session.ok_or(WebError(
        StatusCode::CONFLICT,
        "Start Music in a voice channel first.",
    ))?;
    apply_playback_timer(&state, guild, &session, UserId::new(parse_id(&login.user.id)?), edit).await
}

async fn apply_playback_timer(
    state: &Dashboard,
    guild: GuildId,
    session: &Arc<Session>,
    actor: UserId,
    edit: TimerEdit,
) -> WebResult<Json<serde_json::Value>> {
    let sessions = state.bot.sessions.lock().await;
    let stopped = || WebError(StatusCode::CONFLICT, "Playback has stopped or changed. Try again.");
    if !sessions.get(&guild).is_some_and(|active| Arc::ptr_eq(active, session))
        || !session.active.load(Ordering::SeqCst)
    {
        return Err(stopped());
    }
    let runtime = session.runtime.as_ref().ok_or_else(unavailable)?;
    let same_channel = state
        .cache
        .guild(guild)
        .and_then(|g| g.voice_states.get(&actor).and_then(|s| s.channel_id))
        .is_some_and(|c| c == runtime.channel);
    if !same_channel {
        return Err(WebError(
            StatusCode::FORBIDDEN,
            "Join the same voice channel as Music to control its timer.",
        ));
    }
    if edit.minutes.is_some_and(|m| !(1..=720).contains(&m))
        || edit.cancel && edit.minutes.is_some()
    {
        return Err(WebError(
            StatusCode::BAD_REQUEST,
            "Choose 1–720 minutes or cancel.",
        ));
    }
    let deadline = runtime::set_timer(session, edit.minutes, edit.cancel)
        .await
        .map_err(storage_error)?
        .ok_or_else(stopped)?;
    drop(sessions);
    Ok(Json(serde_json::json!({"sleep_until":deadline})))
}

fn storage_error(_: Error) -> WebError {
    WebError(
        StatusCode::INTERNAL_SERVER_ERROR,
        "Music data could not be saved. Try again.",
    )
}

/// Voice and text channels the member can view, after role and member overwrites.
fn visible_channels<'a>(
    channels: impl IntoIterator<Item = &'a GuildChannel>,
    member: &Member,
    permissions_in: impl Fn(&GuildChannel, &Member) -> Permissions,
) -> Vec<&'a GuildChannel> {
    channels
        .into_iter()
        .filter(|c| matches!(c.kind, ChannelType::Voice | ChannelType::Text))
        .filter(|c| permissions_in(c, member).contains(Permissions::VIEW_CHANNEL))
        .collect()
}

/// New Auto-join and announcement choices must be visible to the saving manager;
/// an unchanged channel chosen earlier by someone else stays valid.
fn chosen_channels_visible(
    new: &ServerSettings,
    stored: &ServerSettings,
    visible: &std::collections::BTreeSet<u64>,
) -> bool {
    [
        (new.voice_channel, stored.voice_channel),
        (new.announcement_channel, stored.announcement_channel),
    ]
    .into_iter()
    .all(|(chosen, before)| chosen.is_none_or(|id| visible.contains(&id) || Some(id) == before))
}

async fn expanded_settings(
    State(state): State<Arc<Dashboard>>,
    headers: HeaderMap,
    RoutePath(value): RoutePath<String>,
) -> WebResult<Json<serde_json::Value>> {
    let login = state.login(&headers, false).await?;
    let guild = GuildId::new(parse_id(&value)?);
    let (server, member) = state.manager_member(guild, &login).await?;
    let access = state.music_access(guild, &login).await?;
    let settings = state
        .bot
        .settings
        .lock()
        .await
        .db
        .settings(guild.get())
        .map_err(storage_error)?;
    let roles: Vec<_> = server
        .roles
        .values()
        .filter(|r| r.id.get() != guild.get())
        .map(|r| serde_json::json!({"id":r.id.to_string(),"name":r.name}))
        .collect();
    let all = guild.channels(&state.http).await.map_err(|_| unavailable())?;
    let channels = visible_channels(all.values(), &member, |c, m| server.user_permissions_in(c, m))
        .into_iter()
        .map(|c| serde_json::json!({"id":c.id.to_string(),"name":c.name,"voice":c.kind==ChannelType::Voice}))
        .collect::<Vec<_>>();
    let session=state.bot.sessions.lock().await.get(&guild).map(|s|serde_json::json!({"station":state.bot.catalog.stations[s.station].id,"track":state.bot.catalog.stations[s.station].tracks[s.current.load(Ordering::SeqCst)].title,"active":s.active.load(Ordering::SeqCst),"paused":s.paused.load(Ordering::SeqCst),"volume":s.volume.load(Ordering::SeqCst),"sleep_until":s.sleep_until.load(Ordering::SeqCst)}));
    Ok(Json(
        serde_json::json!({"settings":settings,"roles":roles,"channels":channels,"session":session,
            "music_access":{"plan":access.name(),"personal":access.personal(),"shared":access.shared(),
                "radio_vote":state.bot.radio_vote_applies(access)}}),
    ))
}

async fn save_expanded(
    State(state): State<Arc<Dashboard>>,
    headers: HeaderMap,
    RoutePath(value): RoutePath<String>,
    Json(settings): Json<ServerSettings>,
) -> WebResult<Json<serde_json::Value>> {
    let login = state.login(&headers, true).await?;
    let guild = GuildId::new(parse_id(&value)?);
    let (server, member) = state.manager_member(guild, &login).await?;
    let access = state.music_access(guild, &login).await?;
    settings.validate(guild.get()).map_err(|_| {
        WebError(
            StatusCode::BAD_REQUEST,
            "Invalid settings or missing Auto-join channel.",
        )
    })?;
    if !state
        .bot
        .catalog
        .stations
        .iter()
        .any(|s| s.id == settings.default_station)
    {
        return Err(WebError(
            StatusCode::BAD_REQUEST,
            "Choose an available station.",
        ));
    }
    let channels = guild
        .channels(&state.http)
        .await
        .map_err(|_| unavailable())?;
    settings.validate_resources(guild.get(),&server.roles.keys().map(|r|r.get()).collect(),&channels.values().filter(|c|c.kind==ChannelType::Voice).map(|c|c.id.get()).collect(),&channels.values().filter(|c|c.kind==ChannelType::Text).map(|c|c.id.get()).collect(),&state.bot.catalog.stations.iter().map(|s|s.id.clone()).collect()).map_err(|_|WebError(StatusCode::BAD_REQUEST,"A selected channel, role or station is unavailable. Auto-join needs both a voice and text channel."))?;
    let visible = visible_channels(channels.values(), &member, |c, m| {
        server.user_permissions_in(c, m)
    })
    .into_iter()
    .map(|c| c.id.get())
    .collect();
    let stored = state
        .bot
        .settings
        .lock()
        .await
        .db
        .settings(guild.get())
        .map_err(storage_error)?;
    if !chosen_channels_visible(&settings, &stored, &visible) {
        return Err(WebError(
            StatusCode::BAD_REQUEST,
            "Choose Auto-join and announcement channels you can view.",
        ));
    }
    if stored.revision != settings.revision {
        return Err(WebError(
            StatusCode::CONFLICT,
            "Settings changed. Reload before saving.",
        ));
    }
    if !access.shared() && settings.changes_shared_music_controls(&stored) {
        return Err(WebError(
            StatusCode::FORBIDDEN,
            "These server controls require Vozen Premium.",
        ));
    }
    if settings.radio_mode != stored.radio_mode
        && settings.radio_mode != RadioMode::Session
        && !state
            .bot
            .radio_vote_allowed(parse_id(&login.user.id)?, access)
            .await
            .map_err(|_| {
                WebError(
                    StatusCode::SERVICE_UNAVAILABLE,
                    crate::access::VOTE_UNAVAILABLE,
                )
            })?
    {
        return Err(WebError(
            StatusCode::FORBIDDEN,
            crate::access::VOTE_REQUIRED,
        ));
    }
    // Match command lock order: session map, voice call, then settings storage.
    let sessions = state.bot.sessions.lock().await;
    let active = sessions.get(&guild).cloned();
    let call = active
        .as_ref()
        .and_then(|s| s.runtime.as_ref())
        .and_then(|r| r.voice.upgrade())
        .and_then(|v| v.get(guild));
    let voice_guard = if let Some(call) = &call {
        Some(call.lock().await)
    } else {
        None
    };
    let mut store = state.bot.settings.lock().await;
    let previous = store.db.settings(guild.get()).map_err(storage_error)?;
    if previous.revision != settings.revision {
        return Err(WebError(
            StatusCode::CONFLICT,
            "Settings changed. Reload before saving.",
        ));
    }
    let saved = store.save(guild.get(), settings).map_err(storage_error)?;
    let mut warning = None;
    if let Some(s) = &active {
        *s.policy.lock().map_err(|_| unavailable())? = saved.skip.clone();
        s.reset_votes(s.generation.load(Ordering::SeqCst)).map_err(|_| unavailable())?;
        if access.shared() && voice_guard.is_some() && s.active.load(Ordering::SeqCst) {
            let track =
                &state.bot.catalog.stations[s.station].tracks[s.current.load(Ordering::SeqCst)];
            if let Some(h) = s.handle.lock().map_err(|_| unavailable())?.as_ref()
                && h.set_volume(10.0_f32.powf(track.gain_db / 20.0) * saved.volume as f32 / 100.0)
                    .is_err()
            {
                warning = Some(
                    "The current recording could not update its volume. The saved volume applies to future recordings.",
                );
            }
        }
        // Without Premium the session keeps the fixed volume it started with.
        if access.shared() {
            s.volume.store(saved.volume as usize, Ordering::SeqCst);
        }
        s.tips.store(saved.tips, Ordering::SeqCst);
    }
    drop(store);
    drop(voice_guard);
    drop(sessions);
    if let Some(s) = active {
        player::refresh(&state.http, &state.bot.catalog, &s, false).await;
    }
    Ok(Json(
        serde_json::json!({"settings":saved,"warning":warning}),
    ))
}

async fn station_catalog(
    State(state): State<Arc<Dashboard>>,
    headers: HeaderMap,
) -> WebResult<Json<serde_json::Value>> {
    state.login(&headers, false).await?;
    let durations: HashMap<String, f64> = serde_json::from_str(include_str!("../web/track-durations.json"))
        .expect("Bundled track durations are valid JSON");
    let mut proposals: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("../web/station-proposals.json"))
            .expect("Bundled station proposals are valid JSON");
    proposals.retain(|p| !state.bot.catalog.stations.iter().any(|s| Some(s.id.as_str()) == p["id"].as_str()));
    Ok(Json(
        serde_json::json!({"stations":state.bot.catalog.stations.iter().map(|s|serde_json::json!({"id":s.id,"name":s.name,"cover":format!("/music/albums/{}-v1.webp",s.id),"tracks":s.tracks.iter().map(|t|serde_json::json!({"id":t.id,"title":t.title,"artist":t.artist,"source":t.source_url,"license":t.license,"duration_seconds":durations.get(&t.id),"thumbnail":t.cover.as_ref().and_then(|p| p.file_stem()).and_then(|n|n.to_str()).map(|n|format!("/music/thumbnails/{n}.webp"))})).collect::<Vec<_>>()})).collect::<Vec<_>>(),"pending":(["night","rain","sleep"].into_iter().filter(|id| !state.bot.catalog.stations.iter().any(|s| s.id == *id)).collect::<Vec<_>>()),"proposals":proposals}),
    ))
}

async fn track_listening(
    State(state): State<Arc<Dashboard>>,
    headers: HeaderMap,
    RoutePath(value): RoutePath<String>,
) -> WebResult<Json<serde_json::Value>> {
    let login = state.login(&headers, false).await?;
    let guild = GuildId::new(parse_id(&value)?);
    state.manager(guild, &login).await?;
    let store = state.bot.settings.lock().await;
    let enabled = store
        .db
        .settings(guild.get())
        .map_err(storage_error)?
        .statistics;
    let top = if enabled {
        store.db.top_track(guild.get()).map_err(storage_error)?
    } else {
        None
    };
    drop(store);
    let top = top.map(|(id, seconds, total_seconds)| {
        let recording = state.bot.catalog.stations.iter().find_map(|station| {
            station.tracks.iter().find(|track| track.id == id).map(|track| (station, track))
        });
        let (title, artist, cover) = recording.map_or_else(
            || ("Unavailable track", None, None),
            |(station, track)| {
                let cover = track.cover.as_ref().and_then(|path| path.file_name())
                    .and_then(|name| name.to_str())
                    .map(|name| format!("/music/covers/{name}"))
                    .unwrap_or_else(|| format!("/music/albums/{}-v1.webp", station.id));
                (track.title.as_str(), Some(track.artist.as_str()), Some(cover))
            },
        );
        serde_json::json!({"title":title,"artist":artist,"cover":cover,"seconds":seconds,"total_seconds":total_seconds})
    });
    Ok(Json(serde_json::json!({"enabled":enabled,"top":top})))
}

async fn personal_member(
    state: &Dashboard,
    headers: &HeaderMap,
    guild: GuildId,
    write: bool,
) -> WebResult<Login> {
    let login = state.login(headers, write).await?;
    if state.bot.guild.is_some_and(|g| g != guild) || state.cache.guild(guild).is_none() {
        return Err(WebError(
            StatusCode::NOT_FOUND,
            "Music is not installed in this server.",
        ));
    }
    guild
        .member(&state.http, UserId::new(parse_id(&login.user.id)?))
        .await
        .map_err(|_| {
            WebError(
                StatusCode::FORBIDDEN,
                "Server membership could not be verified.",
            )
        })?;
    Ok(login)
}

async fn personal(
    State(state): State<Arc<Dashboard>>,
    headers: HeaderMap,
    RoutePath(value): RoutePath<String>,
) -> WebResult<Json<serde_json::Value>> {
    let g = GuildId::new(parse_id(&value)?);
    let login = personal_member(&state, &headers, g, false).await?;
    let u = parse_id(&login.user.id)?;
    let store = state.bot.settings.lock().await;
    let (privacy, seconds) = store.db.privacy(g.get(), u).map_err(storage_error)?;
    let ratings = store
        .db
        .ratings(g.get(), &std::collections::BTreeSet::from([u]))
        .map_err(storage_error)?;
    Ok(Json(
        serde_json::json!({"privacy":privacy,"seconds":seconds,"ratings":ratings.get(&u)}),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PersonalEdit {
    privacy: Option<Privacy>,
    erase_statistics: Option<bool>,
    clear_ratings: Option<bool>,
    track: Option<String>,
    rating: Option<i8>,
}
async fn save_personal(
    State(state): State<Arc<Dashboard>>,
    headers: HeaderMap,
    RoutePath(value): RoutePath<String>,
    Json(edit): Json<PersonalEdit>,
) -> WebResult<Json<serde_json::Value>> {
    let g = GuildId::new(parse_id(&value)?);
    let login = personal_member(&state, &headers, g, true).await?;
    let u = parse_id(&login.user.id)?;
    if let Some(id) = &edit.track
        && (!state
            .bot
            .catalog
            .stations
            .iter()
            .flat_map(|s| &s.tracks)
            .any(|t| &t.id == id)
            || !edit.rating.is_some_and(|r| (-1..=1).contains(&r)))
    {
        return Err(WebError(
            StatusCode::BAD_REQUEST,
            "Choose a valid recording and rating.",
        ));
    }
    let store = state.bot.settings.lock().await;
    if let Some(id) = &edit.track {
        store
            .db
            .rate(g.get(), u, id, edit.rating.unwrap_or(0))
            .map_err(storage_error)?;
    }
    if edit.clear_ratings == Some(true) {
        store.db.clear_ratings(g.get(), u).map_err(storage_error)?;
    }
    let erase = edit.erase_statistics == Some(true);
    if edit.privacy.is_some() || erase {
        store
            .db
            .set_privacy(
                g.get(),
                u,
                if erase {
                    Privacy::default()
                } else {
                    edit.privacy.unwrap_or_default()
                },
                erase,
            )
            .map_err(storage_error)?;
    }
    Ok(Json(serde_json::json!({"saved":true})))
}

async fn community(
    State(state): State<Arc<Dashboard>>,
    headers: HeaderMap,
    RoutePath(value): RoutePath<String>,
) -> WebResult<Json<serde_json::Value>> {
    let guild = GuildId::new(parse_id(&value)?);
    personal_member(&state, &headers, guild, false).await?;
    let store = state.bot.settings.lock().await;
    let rows = if store
        .db
        .settings(guild.get())
        .map_err(storage_error)?
        .statistics
    {
        store.db.leaderboard(guild.get()).map_err(storage_error)?
    } else {
        vec![]
    };
    Ok(Json(serde_json::json!({"rows":rows})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileEdit {
    nick: Option<String>,
    avatar: Option<String>,
    banner: Option<String>,
    reset: Option<bool>,
}
const PROFILE_POLICY: &str = include_str!("../web/profile-policy.json");

fn normalized_profile_text(value: &str) -> String {
    value
        .chars()
        .flat_map(char::to_lowercase)
        .filter_map(|c| {
            let c = match c {
                '\u{ff01}'..='\u{ff5e}' => char::from_u32(c as u32 - 0xfee0).unwrap(),
                other => other,
            };
            Some(match c {
                '0' | 'ο' | 'о' | 'ò' | 'ó' | 'ô' | 'õ' | 'ö' => 'o',
                '1' | '!' | 'і' | 'í' | 'ì' | 'î' | 'ï' => 'i',
                '3' | 'е' | 'é' | 'è' | 'ê' | 'ë' => 'e',
                '4' | '@' | 'а' | 'à' | 'á' | 'â' | 'ã' | 'ä' => 'a',
                '5' | '$' => 's',
                '7' | 'τ' | 'т' => 't',
                'ç' | 'с' => 'c',
                'ù' | 'ú' | 'û' | 'ü' => 'u',
                'ñ' => 'n',
                'ɡ' => 'g',
                c if c.is_ascii_alphanumeric() => c,
                _ => return None,
            })
        })
        .collect()
}

fn validate_profile_text(edit: &ProfileEdit) -> WebResult<()> {
    if edit.reset == Some(true) {
        return Ok(());
    }
    if edit.nick.as_ref().is_some_and(|v| {
        v.chars().count() > 32
            || (!v.is_empty() && v.trim().is_empty())
            || v.chars().any(|c| c.is_control())
    }) {
        return Err(WebError(
            StatusCode::BAD_REQUEST,
            "Use a display name up to 32 characters without line breaks. Leave the name empty to use Vozen Music.",
        ));
    }
    let policy: serde_json::Value =
        serde_json::from_str(PROFILE_POLICY).expect("Bundled profile policy is valid JSON");
    if let Some(text) = edit.nick.as_deref() {
        let compact = normalized_profile_text(text);
        let words: Vec<_> = text
            .split(|c: char| !c.is_alphanumeric())
            .map(normalized_profile_text)
            .collect();
        if policy["blocked_terms"]
            .as_array()
            .unwrap()
            .iter()
            .any(|term| compact.contains(term.as_str().unwrap()))
            || policy["blocked_words"]
                .as_array()
                .unwrap()
                .iter()
                .any(|term| {
                    words.iter().any(|word| word == term.as_str().unwrap())
                        || compact == term.as_str().unwrap()
                })
        {
            return Err(WebError(
                StatusCode::BAD_REQUEST,
                "Choose a different name or description. Discriminatory slurs and hate slogans are not allowed.",
            ));
        }
    }
    Ok(())
}
fn animated_image(bytes: &[u8]) -> bool {
    let png = bytes.starts_with(b"\x89PNG\r\n\x1a\n");
    let webp = bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP";
    if !png && !webp {
        return false;
    }
    let mut offset = if png { 8 } else { 12 };
    while offset + 12 <= bytes.len() {
        let (kind, length) = if png {
            (
                &bytes[offset + 4..offset + 8],
                u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize,
            )
        } else {
            (
                &bytes[offset..offset + 4],
                u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap()) as usize,
            )
        };
        if png && kind == b"acTL" || webp && (kind == b"ANIM" || kind == b"ANMF") {
            return true;
        }
        let overhead = if png { 12 } else { 8 + (length % 2) };
        let Some(next) = offset
            .checked_add(length)
            .and_then(|n| n.checked_add(overhead))
        else {
            return false;
        };
        offset = next;
    }
    false
}

fn profile_image(data: &str) -> Result<String, Error> {
    use base64::{Engine, engine::general_purpose::STANDARD};
    if data.len() > 2_800_000 {
        return Err("Image exceeds 2 MB.".into());
    }
    let encoded = data
        .split_once(",")
        .filter(|(p, _)| {
            matches!(
                *p,
                "data:image/png;base64" | "data:image/jpeg;base64" | "data:image/webp;base64"
            )
        })
        .ok_or("Upload a PNG, JPEG or WebP image.")?
        .1;
    let bytes = STANDARD.decode(encoded)?;
    if bytes.len() > 2 * 1024 * 1024 || animated_image(&bytes) {
        return Err("Choose a static image up to 2 MB.".into());
    }
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader.decode()?;
    let mut png = std::io::Cursor::new(Vec::new());
    decoded.write_to(&mut png, image::ImageFormat::Png)?;
    if png.get_ref().len() > 2 * 1024 * 1024 {
        return Err("The processed image exceeds 2 MB. Use a smaller image.".into());
    }
    Ok(format!(
        "data:image/png;base64,{}",
        STANDARD.encode(png.into_inner())
    ))
}
async fn bot_profile(
    State(state): State<Arc<Dashboard>>,
    headers: HeaderMap,
    RoutePath(value): RoutePath<String>,
) -> WebResult<Json<serde_json::Value>> {
    let login = state.login(&headers, false).await?;
    let g = GuildId::new(parse_id(&value)?);
    state.manager(g, &login).await?;
    let bot_id = state.cache.current_user().id;
    let response = state
        .client
        .get(format!("{API}/guilds/{g}/members/{bot_id}"))
        .header("Authorization", state.http.token())
        .send()
        .await
        .map_err(|_| unavailable())?;
    if !response.status().is_success() {
        return Err(unavailable());
    }
    let member: serde_json::Value = response.json().await.map_err(|_| unavailable())?;
    Ok(Json(
        serde_json::json!({"nick":member["nick"],"bio":member["bio"],"avatar":member["avatar"],"banner":member["banner"],"bot_id":bot_id.to_string(),"default_name":state.cache.current_user().name,"default_avatar_url":state.cache.current_user().avatar_url().unwrap_or_else(||"/music/avatar.png".into()),"policy":serde_json::from_str::<serde_json::Value>(PROFILE_POLICY).map_err(|_|unavailable())?}),
    ))
}
async fn save_bot_profile(
    State(state): State<Arc<Dashboard>>,
    headers: HeaderMap,
    RoutePath(value): RoutePath<String>,
    Json(edit): Json<ProfileEdit>,
) -> WebResult<Json<serde_json::Value>> {
    let login = state.login(&headers, true).await?;
    let g = GuildId::new(parse_id(&value)?);
    state.manager(g, &login).await?;
    if edit.reset != Some(true)
        && (edit.avatar.is_some() || edit.banner.is_some())
        && !state.music_access(g, &login).await?.shared()
    {
        return Err(WebError(StatusCode::FORBIDDEN, "Custom bot artwork requires Vozen Premium."));
    }
    validate_profile_text(&edit)?;
    let mut body = serde_json::Map::new();
    if edit.reset == Some(true) {
        for field in ["nick", "avatar", "banner"] {
            body.insert(field.into(), serde_json::Value::Null);
        }
    } else {
        if let Some(v) = edit.nick {
            body.insert(
                "nick".into(),
                if v.is_empty() {
                    serde_json::Value::Null
                } else {
                    v.into()
                },
            );
        }
        for (field, data) in [("avatar", edit.avatar), ("banner", edit.banner)] {
            if let Some(data) = data {
                let image=tokio::task::spawn_blocking(move||profile_image(&data)).await.map_err(|_|unavailable())?.map_err(|_|WebError(StatusCode::BAD_REQUEST,"Choose a valid static PNG, JPEG or WebP image up to 2 MB and 4096 pixels."))?;
                body.insert(field.into(), image.into());
            }
        }
    }
    let response = state
        .client
        .patch(format!("{API}/guilds/{g}/members/@me"))
        .header("Authorization", state.http.token())
        .json(&body)
        .send()
        .await
        .map_err(|_| unavailable())?;
    if !response.status().is_success() {
        return Err(WebError(
            StatusCode::BAD_GATEWAY,
            "Discord did not apply the profile. Check permissions and try again later.",
        ));
    }
    Ok(Json(serde_json::json!({"saved":true})))
}

#[cfg(test)]
mod expansion_web_tests {
    use super::*;
    #[tokio::test]
    async fn dashboard_timer_rejects_stopped_and_replaced_sessions() {
        let state = tests::fixture();
        let guild = GuildId::new(1);
        let actor = UserId::new(123);
        let channel = ChannelId::new(2);
        let mut cached_guild = Guild::default();
        cached_guild.id = guild;
        cached_guild.voice_states.insert(actor, serde_json::from_value(serde_json::json!({
            "guild_id":"1","channel_id":"2","user_id":"123","session_id":"fixture",
            "deaf":false,"mute":false,"self_deaf":false,"self_mute":false,"self_video":false,"suppress":false
        })).unwrap());
        let mut event: GuildCreateEvent =
            serde_json::from_value(serde_json::to_value(cached_guild).unwrap()).unwrap();
        state.cache.update(&mut event);
        let legacy = std::env::temp_dir().join(format!("music-timer-{}.json", nonce()));
        let settings = Arc::new(Mutex::new(SettingsStore::load(legacy.clone()).unwrap()));
        let mut session = Session::new(0, 0, 1, SkipSettings::default(), None);
        session.runtime = Some(Arc::new(runtime::Runtime {
            guild,
            channel,
            text: channel,
            cache: state.cache.clone(),
            http: state.http.clone(),
            settings: settings.clone(),
            voice: Weak::new(),
            quality: Arc::default(),
        }));
        let session = Arc::new(session);
        let mut intent = vozen_music::expansion::PlaybackIntent {
            guild: guild.get(),
            channel: channel.get(),
            text_channel: channel.get(),
            station: "lofi".into(),
            track: "first".into(),
            active: true,
            sleep_until: None,
        };
        settings.lock().await.db.intent(&intent).unwrap();
        state.bot.sessions.lock().await.insert(guild, session.clone());
        let update = || TimerEdit { minutes: Some(1), cancel: false };
        let response = apply_playback_timer(&state, guild, &session, actor, update())
            .await
            .ok()
            .unwrap();
        let deadline = response.0["sleep_until"].as_u64().unwrap();
        assert_eq!(settings.lock().await.db.intents().unwrap()[0].sleep_until, Some(deadline as i64));
        assert_eq!(settings.lock().await.db.intents().unwrap()[0].track, "first");

        let connection = rusqlite::Connection::open(legacy.with_extension("sqlite3")).unwrap();
        connection.execute_batch("CREATE TRIGGER reject_timer BEFORE UPDATE ON playback
            BEGIN SELECT RAISE(ABORT, 'test write failure'); END;").unwrap();
        assert_eq!(runtime::set_timer(&session, None, false).await.unwrap(), Some(deadline as usize));
        assert!(runtime::set_timer(&session, None, true).await.is_err());
        assert_eq!(session.sleep_until.load(Ordering::SeqCst), deadline as usize);
        assert_eq!(settings.lock().await.db.intents().unwrap()[0].sleep_until, Some(deadline as i64));
        connection.execute_batch("DROP TRIGGER reject_timer").unwrap();

        for replace in [false, true] {
            let mut sessions = state.bot.sessions.lock().await;
            sessions.insert(guild, session.clone());
            session.active.store(true, Ordering::SeqCst);
            let request = apply_playback_timer(&state, guild, &session, actor, update());
            tokio::pin!(request);
            tokio::select! {
                biased;
                _ = &mut request => panic!("Timer completed while the session lock was held"),
                _ = std::future::ready(()) => {}
            }
            session.active.store(false, Ordering::SeqCst);
            intent.active = replace;
            intent.track = if replace { "replacement" } else { "stopped" }.into();
            settings.lock().await.db.intent(&intent).unwrap();
            if replace {
                sessions.insert(guild, Arc::new(Session::new(0, 0, 2, SkipSettings::default(), None)));
            } else {
                sessions.remove(&guild);
            }
            drop(sessions);
            assert_eq!(request.await.err().unwrap().0, StatusCode::CONFLICT);
            let stored = &settings.lock().await.db.intents().unwrap()[0];
            assert_eq!(stored.active, replace);
            assert_eq!(stored.track, intent.track);
            assert_eq!(stored.sleep_until, None);
        }
    }

    #[test]
    fn profile_text_rejects_slurs_before_discord_and_preserves_normal_names() {
        for name in [
            "nigger",
            "NIGGER",
            "n1gg3r",
            "n.i.g.g.e.r",
            "n i g g e r",
            "ｎｉｇｇｅｒ",
            "n\u{200b}igger",
            "nіggеr",
            "faggot",
            "white power",
            "   ",
            "Music\nBot",
            &"x".repeat(33),
        ] {
            let edit: ProfileEdit =
                serde_json::from_value(serde_json::json!({"nick":name})).unwrap();
            assert!(
                validate_profile_text(&edit).is_err(),
                "Invalid profile text was accepted"
            );
        }
        for name in [
            "",
            "Vozen Music",
            "Night Radio",
            "Nigel",
            "Niger Music",
            "Pakistan Radio",
            "Scunthorpe",
            "Música da chuva",
            "音楽",
            "DJ 🌧️",
        ] {
            let edit: ProfileEdit =
                serde_json::from_value(serde_json::json!({"nick":name})).unwrap();
            assert!(
                validate_profile_text(&edit).is_ok(),
                "Ordinary profile text was rejected"
            );
        }
        let reset: ProfileEdit =
            serde_json::from_value(serde_json::json!({"reset":true,"nick":"nigger"})).unwrap();
        assert!(validate_profile_text(&reset).is_ok());
    }
    #[test]
    fn profile_description_cannot_be_changed_or_reset() {
        for payload in [
            serde_json::json!({"bio":"A new description"}),
            serde_json::json!({"bio":null}),
            serde_json::json!({"reset":true,"bio":""}),
        ] {
            assert!(serde_json::from_value::<ProfileEdit>(payload).is_err());
        }
    }
    use tower::ServiceExt;
    #[tokio::test]
    async fn every_new_resource_requires_a_session_and_csrf() {
        use axum::{body::Body, http::Request};
        let app = router(super::tests::fixture());
        for suffix in ["server-settings", "personal", "community", "profile"] {
            let path = format!("/music/api/guilds/1524199248248176751/{suffix}");
            let r = app
                .clone()
                .oneshot(Request::builder().uri(&path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(r.status(), StatusCode::UNAUTHORIZED, "{suffix}");
            if suffix != "community" {
                let r = app
                    .clone()
                    .oneshot(
                        Request::builder()
                            .uri(&path)
                            .method("PUT")
                            .header("content-type", "application/json")
                            .header("cookie", "__Secure-vozen_music=test-session")
                            .body(Body::from("{}"))
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                assert_eq!(r.status(), StatusCode::FORBIDDEN, "{suffix}");
            }
        }
        let r = app
            .oneshot(
                Request::builder()
                    .uri("/music/api/guilds/1524199248248176751/playback")
                    .method("PUT")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"cancel":true}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    }
    #[tokio::test]
    async fn profile_authentication_precedes_json_parsing() {
        use axum::{body::Body, http::Request};
        let app = router(super::tests::fixture());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/music/api/guilds/1524199248248176751/profile")
                    .method("PUT")
                    .header("content-type", "application/json")
                    .body(Body::from("not JSON"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
    #[test]
    fn static_jpeg_comments_are_not_mistaken_for_animation() {
        use base64::{Engine, engine::general_purpose::STANDARD};
        let image = image::DynamicImage::new_rgb8(2, 2);
        let mut output = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut output, image::ImageFormat::Jpeg)
            .unwrap();
        let bytes = output.into_inner();
        let mut commented = bytes[..2].to_vec();
        commented.extend_from_slice(b"\xff\xfe\x00\x06ANIM");
        commented.extend_from_slice(&bytes[2..]);
        let input = format!("data:image/jpeg;base64,{}", STANDARD.encode(commented));
        assert!(profile_image(&input).is_ok());
    }
    #[test]
    fn image_processing_rejects_urls_corrupt_and_oversized_data() {
        assert!(profile_image("https://example.com/image.png").is_err());
        assert!(profile_image("data:image/png;base64,YmFk").is_err());
        assert!(profile_image(&"a".repeat(2_800_001)).is_err());
        use base64::{Engine, engine::general_purpose::STANDARD};
        let image = image::DynamicImage::new_rgb8(2, 2);
        let mut png = std::io::Cursor::new(Vec::new());
        image.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let data = format!(
            "data:image/png;base64,{}",
            STANDARD.encode(png.into_inner())
        );
        assert!(
            profile_image(&data)
                .unwrap()
                .starts_with("data:image/png;base64,")
        );
    }
}
