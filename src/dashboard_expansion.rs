use vozen_music::expansion::{Privacy, ServerSettings};
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
    state.manager(guild, &login).await?;
    let session = state
        .bot
        .sessions
        .lock()
        .await
        .get(&guild)
        .cloned()
        .ok_or(WebError(
            StatusCode::CONFLICT,
            "Start Music in a voice channel first.",
        ))?;
    let runtime = session.runtime.as_ref().ok_or_else(unavailable)?;
    let actor = UserId::new(parse_id(&login.user.id)?);
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
    runtime::set_timer(&state.bot.catalog, &session, edit.minutes, edit.cancel)
        .await
        .map_err(storage_error)?;
    Ok(Json(
        serde_json::json!({"sleep_until":session.sleep_until.load(Ordering::SeqCst)}),
    ))
}

fn storage_error(_: Error) -> WebError {
    WebError(
        StatusCode::INTERNAL_SERVER_ERROR,
        "Music data could not be saved. Try again.",
    )
}

async fn expanded_settings(
    State(state): State<Arc<Dashboard>>,
    headers: HeaderMap,
    RoutePath(value): RoutePath<String>,
) -> WebResult<Json<serde_json::Value>> {
    let login = state.login(&headers, false).await?;
    let guild = GuildId::new(parse_id(&value)?);
    let server = state.manager(guild, &login).await?;
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
    let channels=guild.channels(&state.http).await.map_err(|_|unavailable())?.values().filter(|c|matches!(c.kind,ChannelType::Voice|ChannelType::Text)).map(|c|serde_json::json!({"id":c.id.to_string(),"name":c.name,"voice":c.kind==ChannelType::Voice})).collect::<Vec<_>>();
    let session=state.bot.sessions.lock().await.get(&guild).map(|s|serde_json::json!({"station":state.bot.catalog.stations[s.station].id,"track":state.bot.catalog.stations[s.station].tracks[s.current.load(Ordering::SeqCst)].title,"paused":s.paused.load(Ordering::SeqCst),"volume":s.volume.load(Ordering::SeqCst),"sleep_until":s.sleep_until.load(Ordering::SeqCst)}));
    Ok(Json(
        serde_json::json!({"settings":settings,"roles":roles,"channels":channels,"session":session}),
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
    let server = state.manager(guild, &login).await?;
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
    if store
        .db
        .settings(guild.get())
        .map_err(storage_error)?
        .revision
        != settings.revision
    {
        return Err(WebError(
            StatusCode::CONFLICT,
            "Settings changed. Reload before saving.",
        ));
    }
    let saved = store.save(guild.get(), settings).map_err(storage_error)?;
    let mut warning = None;
    if let Some(s) = &active {
        *s.policy.lock().map_err(|_| unavailable())? = saved.skip.clone();
        s.votes
            .lock()
            .map_err(|_| unavailable())?
            .reset(s.generation.load(Ordering::SeqCst));
        if voice_guard.is_some() && s.active.load(Ordering::SeqCst) {
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
        s.volume.store(saved.volume as usize, Ordering::SeqCst);
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
    Ok(Json(
        serde_json::json!({"stations":state.bot.catalog.stations.iter().map(|s|serde_json::json!({"id":s.id,"name":s.name,"tracks":s.tracks.iter().map(|t|serde_json::json!({"id":t.id,"title":t.title,"artist":t.artist,"source":t.source_url,"license":t.license})).collect::<Vec<_>>()})).collect::<Vec<_>>(),"pending":["night","rain","sleep"]}),
    ))
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
    bio: Option<String>,
    avatar: Option<String>,
    banner: Option<String>,
    reset: Option<bool>,
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
    if bytes.len() > 2 * 1024 * 1024 || bytes.windows(4).any(|w| w == b"acTL" || w == b"ANIM") {
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
        serde_json::json!({"nick":member["nick"],"bio":member["bio"],"avatar":member["avatar"],"banner":member["banner"]}),
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
    if edit.nick.as_ref().is_some_and(|v| v.chars().count() > 32)
        || edit.bio.as_ref().is_some_and(|v| v.chars().count() > 190)
    {
        return Err(WebError(
            StatusCode::BAD_REQUEST,
            "Name: up to 32 characters. Bio: up to 190.",
        ));
    }
    let mut body = serde_json::Map::new();
    if edit.reset == Some(true) {
        for field in ["nick", "bio", "avatar", "banner"] {
            body.insert(field.into(), serde_json::Value::Null);
        }
    } else {
        if let Some(v) = edit.nick {
            body.insert("nick".into(), v.into());
        }
        if let Some(v) = edit.bio {
            body.insert("bio".into(), v.into());
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
