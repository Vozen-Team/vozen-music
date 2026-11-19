use super::*;
use std::collections::BTreeSet;
use vozen_music::expansion::{PlaybackIntent, Privacy, RadioMode, ServerSettings};

pub struct Runtime {
    pub guild: GuildId,
    pub channel: ChannelId,
    pub text: ChannelId,
    pub cache: Arc<Cache>,
    pub http: Arc<Http>,
    pub settings: Arc<Mutex<SettingsStore>>,
    pub voice: Weak<songbird::Songbird>,
}

pub const USER_PLAY_COOLDOWN: Duration = Duration::from_secs(3);
pub const GUILD_PLAY_COOLDOWN: Duration = Duration::from_secs(2);

async fn announce_auto_join(http: &Http, channel: ChannelId, message: String) -> Result<(), Error> {
    channel
        .send_message(
            http,
            CreateMessage::new()
                .content(message)
                .allowed_mentions(CreateAllowedMentions::new()),
        )
        .await?;
    Ok(())
}

/// Throttles /play and /station: starting a session joins voice and posts the
/// player while the shared session map is locked.
#[derive(Default)]
pub struct PlayCooldowns {
    users: HashMap<UserId, std::time::Instant>,
    guilds: HashMap<GuildId, std::time::Instant>,
}

impl PlayCooldowns {
    pub fn begin(
        &mut self,
        guild: GuildId,
        user: UserId,
        now: std::time::Instant,
    ) -> Result<(), Error> {
        self.users
            .retain(|_, at| now.saturating_duration_since(*at) < USER_PLAY_COOLDOWN);
        self.guilds
            .retain(|_, at| now.saturating_duration_since(*at) < GUILD_PLAY_COOLDOWN);
        if self.users.contains_key(&user) {
            return Err("Please wait a few seconds before changing stations again.".into());
        }
        if self.guilds.contains_key(&guild) {
            return Err(
                "Music is already changing stations in this server. Try again shortly.".into(),
            );
        }
        self.users.insert(user, now);
        self.guilds.insert(guild, now);
        Ok(())
    }
}

pub async fn set_timer(
    session: &Session,
    minutes: Option<i64>,
    cancel: bool,
) -> Result<Option<usize>, Error> {
    if cancel && minutes.is_some() {
        return Err("Choose a duration or cancel, not both.".into());
    }
    if minutes.is_some_and(|m| !(1..=720).contains(&m)) {
        return Err("Choose 1 to 720 minutes.".into());
    }
    if !session.active.load(Ordering::SeqCst) {
        return Ok(None);
    }
    let deadline = match (cancel, minutes) {
        (true, _) => 0,
        (false, Some(minutes)) => (chrono::Utc::now().timestamp() + minutes * 60) as usize,
        (false, None) => return Ok(Some(session.sleep_until.load(Ordering::SeqCst))),
    };
    if let Some(runtime) = &session.runtime {
        let store = runtime.settings.lock().await;
        if !session.active.load(Ordering::SeqCst)
            || !store.db.update_active_timer(
                runtime.guild.get(),
                (deadline != 0).then_some(deadline as i64),
            )?
        {
            return Ok(None);
        }
        session.sleep_until.store(deadline, Ordering::SeqCst);
    } else {
        session.sleep_until.store(deadline, Ordering::SeqCst);
    }
    Ok(Some(deadline))
}

pub fn definitions() -> Vec<CreateCommand> {
    let integer = |name, description, min, max| {
        CreateCommandOption::new(CommandOptionType::Integer, name, description)
            .min_int_value(min)
            .max_int_value(max)
    };
    let mut commands = vec![
        CreateCommand::new("volume")
            .description("View or set the playback volume")
            .add_option(integer("level", "Volume from 0 to 100", 0, 100)),
        CreateCommand::new("sleep")
            .description("View, set or cancel the sleep timer")
            .add_option(integer("minutes", "Stop after this many minutes", 1, 720))
            .add_option(CreateCommandOption::new(
                CommandOptionType::Boolean,
                "cancel",
                "Cancel the timer",
            )),
        CreateCommand::new("library")
            .description("View or remove your ratings in this server")
            .add_option(integer("page", "Library page", 1, 10000))
            .add_option(CreateCommandOption::new(
                CommandOptionType::String,
                "remove",
                "Recording ID to remove",
            ))
            .add_option(CreateCommandOption::new(
                CommandOptionType::Boolean,
                "clear",
                "Delete all your ratings in this server",
            )),
        CreateCommand::new("privacy")
            .description("Control your server listening statistics")
            .add_option(CreateCommandOption::new(
                CommandOptionType::Boolean,
                "enabled",
                "Opt in to your time in calls and unnamed server track totals",
            ))
            .add_option(CreateCommandOption::new(
                CommandOptionType::Boolean,
                "visible",
                "Appear on this server leaderboard",
            ))
            .add_option(CreateCommandOption::new(
                CommandOptionType::Boolean,
                "erase",
                "Erase your listening statistics",
            )),
        CreateCommand::new("radio-mode")
            .description("Choose how Music behaves in empty calls")
            .default_member_permissions(Permissions::MANAGE_GUILD)
            .add_option(
                CreateCommandOption::new(CommandOptionType::String, "mode", "Radio mode")
                    .add_string_choice("Session", "session")
                    .add_string_choice("24/7", "always")
                    .add_string_choice("Auto-join", "auto_join"),
            )
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::Channel,
                    "channel",
                    "Voice channel for Auto-join",
                )
                .channel_types(vec![ChannelType::Voice]),
            ),
    ];
    for (name, description) in [
        ("like", "Like this recording"),
        ("dislike", "Dislike this recording"),
        ("panel", "Show a fresh player panel"),
        ("profile", "View your time-in-call profile"),
        ("leaderboard", "View this server's opted-in leaderboard"),
        ("settings", "Open this server's Music settings"),
        ("help", "Show Music commands"),
        ("support", "Get Vozen support"),
        ("invite", "Add Vozen Music to a server"),
        ("feedback", "Find where to send feedback"),
    ] {
        commands.push(CreateCommand::new(name).description(description));
    }
    commands
}

pub async fn humans(
    cache: &Cache,
    http: &Http,
    guild: GuildId,
    channel: ChannelId,
    statistics: bool,
) -> Result<BTreeSet<u64>, Error> {
    let states: Vec<_> = {
        let g = cache.guild(guild).ok_or("The server is still loading.")?;
        g.voice_states
            .iter()
            .filter(|(_, s)| {
                s.channel_id == Some(channel) && (!statistics || !(s.deaf || s.self_deaf))
            })
            .map(|(u, state)| {
                (
                    *u,
                    state
                        .member
                        .as_ref()
                        .map(|m| m.user.bot)
                        .or_else(|| g.members.get(u).map(|m| m.user.bot)),
                )
            })
            .collect()
    };
    let mut result = BTreeSet::new();
    for (user, member_bot) in states {
        let known = member_bot.or_else(|| cache.user(user).map(|u| u.bot));
        let bot = match known {
            Some(b) => b,
            None => guild.member(http, user).await?.user.bot,
        };
        if !bot {
            result.insert(user.get());
        }
    }
    Ok(result)
}

impl Runtime {
    pub async fn allows_recording(&self, id: &str) -> Result<bool, Error> {
        let people = humans(&self.cache, &self.http, self.guild, self.channel, false).await?;
        let ratings = self
            .settings
            .lock()
            .await
            .db
            .ratings(self.guild.get(), &people)?;
        Ok(!ratings.values().any(|r| r.get(id) == Some(&-1)))
    }

    pub async fn next(&self, catalog: &Catalog, session: &Session) -> Result<Option<usize>, Error> {
        let people = humans(&self.cache, &self.http, self.guild, self.channel, false).await?;
        let ratings = self
            .settings
            .lock()
            .await
            .db
            .ratings(self.guild.get(), &people)?;
        let tracks = &catalog.stations[session.station].tracks;
        let ids: Vec<_> = tracks.iter().map(|t| t.id.clone()).collect();
        let current = session.current.load(Ordering::SeqCst);
        let eligible = vozen_music::expansion::candidates(
            &ids,
            &ratings,
            tracks.get(current).map(|t| t.id.as_str()),
            catalog.stations[session.station].id == "my",
            session.turn.load(Ordering::SeqCst),
        );
        if eligible.is_empty() {
            return Ok(None);
        }
        let total: f64 = eligible.iter().map(|(_, w)| w).sum();
        let mut draw = rand::random::<f64>() * total;
        for (id, weight) in &eligible {
            draw -= weight;
            if draw <= 0.0 {
                return Ok(ids.iter().position(|t| t == id));
            }
        }
        Ok(ids
            .iter()
            .position(|t| t == &eligible[eligible.len() - 1].0))
    }
    pub async fn persist(
        &self,
        catalog: &Catalog,
        session: &Session,
        active: bool,
    ) -> Result<(), Error> {
        self.settings.lock().await.db.intent(&PlaybackIntent {
            guild: self.guild.get(),
            channel: self.channel.get(),
            text_channel: self.text.get(),
            station: catalog.stations[session.station].id.clone(),
            track: catalog.stations[session.station].tracks[session.current.load(Ordering::SeqCst)]
                .id
                .clone(),
            active,
            sleep_until: match session.sleep_until.load(Ordering::SeqCst) {
                0 => None,
                v => Some(v as i64),
            },
        })
    }
}

impl Bot {
    #[allow(clippy::too_many_arguments)]
    pub async fn start_session(
        &self,
        ctx: &Context,
        guild: GuildId,
        channel: ChannelId,
        text: ChannelId,
        station: usize,
        control: u64,
        restore: Option<&str>,
        sleep: Option<i64>,
    ) -> Result<String, Error> {
        if self.guild.is_some_and(|g| g != guild) {
            return Err("This version is limited to the development server.".into());
        }
        let cfg = self.settings.lock().await.db.settings(guild.get())?;
        if !cfg.voice_rules.permits(channel.get())
            || !self.message_channel_allowed(ctx, guild, &cfg, text)
            || cfg.voice_channel.is_some_and(|id| id != channel.get())
        {
            return Err("This channel is not allowed by the server's Music settings.".into());
        }
        {
            let server = ctx
                .cache
                .guild(guild)
                .ok_or("The server is still loading.")?;
            let c = server
                .channels
                .get(&channel)
                .ok_or("Voice channel unavailable.")?;
            if c.kind != ChannelType::Voice {
                return Err("Choose a standard voice channel.".into());
            }
            #[allow(deprecated)]
            let permissions = c.permissions_for_user(&ctx.cache, ctx.cache.current_user().id)?;
            if !permissions
                .contains(Permissions::VIEW_CHANNEL | Permissions::CONNECT | Permissions::SPEAK)
            {
                return Err("Music needs View Channel, Connect and Speak.".into());
            }
        }
        let manager = songbird::get(ctx)
            .await
            .ok_or("Voice service unavailable.")?;
        let mut sessions = self.sessions.lock().await;
        if restore.is_some()
            && sessions
                .get(&guild)
                .is_some_and(|s| s.active.load(Ordering::SeqCst))
            && let Some(call) = manager.get(guild)
            && call.lock().await.current_channel().is_some()
        {
            return Ok("Music is already connected.".into());
        }
        let call = tokio::time::timeout(Duration::from_secs(15), manager.join(guild, channel))
            .await
            .map_err(|_| "Voice connection timed out.")??;
        let mut s = Session::new(station, 0, control, cfg.skip, None);
        s.volume.store(cfg.volume as usize, Ordering::SeqCst);
        s.tips.store(cfg.tips, Ordering::SeqCst);
        s.sleep_until
            .store(sleep.unwrap_or(0).max(0) as usize, Ordering::SeqCst);
        s.runtime = Some(Arc::new(Runtime {
            guild,
            channel,
            text,
            cache: ctx.cache.clone(),
            http: ctx.http.clone(),
            settings: self.settings.clone(),
            voice: Arc::downgrade(&manager),
        }));
        let index = if let Some(id) = restore {
            self.catalog.stations[station]
                .tracks
                .iter()
                .position(|t| t.id == id)
        } else {
            s.runtime.as_ref().unwrap().next(&self.catalog, &s).await?
        };
        let Some(index) = index else {
            let _ = manager.remove(guild).await;
            return Err(
                "No recordings match the listeners' preferences. Change ratings and use /play."
                    .into(),
            );
        };
        s.current.store(index, Ordering::SeqCst);
        if cfg.announcements {
            match player::post(ctx, &self.catalog, &s).await {
                Ok(panel) => *s.panel.lock().map_err(|_| "Panel unavailable.")? = Some(panel),
                Err(e) => {
                    if !sessions.contains_key(&guild) {
                        let _ = manager.remove(guild).await;
                    }
                    return Err(e);
                }
            }
        }
        let s = Arc::new(s);
        s.runtime
            .as_ref()
            .unwrap()
            .persist(&self.catalog, &s, true)
            .await?;
        let old = sessions.remove(&guild);
        if let Some(old) = &old {
            old.active.store(false, Ordering::SeqCst);
        }
        let mut locked = call.lock().await;
        play_track(
            &mut locked,
            Advance {
                call: Arc::downgrade(&call),
                catalog: self.catalog.clone(),
                session: s.clone(),
                http: ctx.http.clone(),
                text_channel: text,
                generation: 0,
            },
            index,
        )?;
        sessions.insert(guild, s);
        drop(locked);
        drop(sessions);
        if let Some(old) = old {
            player::refresh(&ctx.http, &self.catalog, &old, true).await;
        }
        Ok(format!(
            "Station selected: {}.{}",
            self.catalog.stations[station].name,
            if cfg.announcements {
                format!(" Player controls are in <#{channel}>.")
            } else {
                String::new()
            }
        ))
    }

    pub async fn stop_session(&self, ctx: &Context, guild: GuildId) -> Result<(), Error> {
        let mut sessions = self.sessions.lock().await;
        let mut persistence = Ok(());
        if let Some(s) = sessions.get(&guild) {
            if let Some(r) = &s.runtime {
                persistence = r.persist(&self.catalog, s, false).await;
            }
            s.active.store(false, Ordering::SeqCst);
            player::refresh(&ctx.http, &self.catalog, s, true).await;
        }
        if let Some(manager) = songbird::get(ctx).await {
            tokio::time::timeout(Duration::from_secs(5), manager.remove(guild))
                .await
                .map_err(|_| "Leaving voice timed out.")??;
        }
        let old = sessions.remove(&guild);
        drop(sessions);
        if let Some(s) = old {
            player::refresh(&ctx.http, &self.catalog, &s, true).await;
        }
        persistence
    }

    pub async fn reconcile(
        &self,
        ctx: &Context,
        guild: GuildId,
        entered: bool,
    ) -> Result<(), Error> {
        let cfg = self.settings.lock().await.db.settings(guild.get())?;
        let active = self.sessions.lock().await.get(&guild).cloned();
        if let Some(s) = active {
            if let Some(r) = &s.runtime {
                let bot_id = ctx.cache.current_user().id;
                let actual_channel = ctx
                    .cache
                    .guild(guild)
                    .and_then(|g| g.voice_states.get(&bot_id).and_then(|v| v.channel_id));
                if actual_channel.is_some_and(|channel| channel != r.channel) {
                    self.stop_session(ctx, guild).await?;
                    return Ok(());
                }
                let deadline = s.sleep_until.load(Ordering::SeqCst);
                let empty = humans(&ctx.cache, &ctx.http, guild, r.channel, false)
                    .await?
                    .is_empty();
                if deadline > 0 && deadline <= chrono::Utc::now().timestamp() as usize
                    || cfg.radio_mode != RadioMode::Always && empty
                    || !cfg.voice_rules.permits(r.channel.get())
                    || cfg.voice_channel.is_some_and(|c| c != r.channel.get())
                {
                    self.stop_session(ctx, guild).await?;
                } else {
                    let connected = if let Some(manager) = songbird::get(ctx).await {
                        if let Some(call) = manager.get(guild) {
                            call.lock().await.current_channel().is_some()
                        } else {
                            false
                        }
                    } else {
                        false
                    };
                    if !connected && s.recovery_attempts.fetch_add(1, Ordering::SeqCst) < 3 {
                        let track = &self.catalog.stations[s.station].tracks
                            [s.current.load(Ordering::SeqCst)]
                        .id;
                        self.start_session(
                            ctx,
                            guild,
                            r.channel,
                            r.text,
                            s.station,
                            rand::random::<u64>().max(1),
                            Some(track),
                            if deadline > 0 {
                                Some(deadline as i64)
                            } else {
                                None
                            },
                        )
                        .await?;
                    }
                }
            }
        } else if entered && cfg.radio_mode == RadioMode::AutoJoin {
            let channel = ChannelId::new(
                cfg.voice_channel
                    .ok_or("Auto-join needs a voice channel.")?,
            );
            if !humans(&ctx.cache, &ctx.http, guild, channel, false)
                .await?
                .is_empty()
            {
                let text = cfg
                    .announcement_channel
                    .ok_or("Choose an announcement channel before enabling Auto-join.")?;
                let station = self
                    .catalog
                    .stations
                    .iter()
                    .position(|s| s.id == cfg.default_station)
                    .ok_or("Default station unavailable.")?;
                let message = self
                    .start_session(
                        ctx,
                        guild,
                        channel,
                        ChannelId::new(text),
                        station,
                        rand::random::<u64>().max(1),
                        None,
                        None,
                    )
                    .await?;
                announce_auto_join(&ctx.http, ChannelId::new(text), message).await?;
            }
        }
        Ok(())
    }

    pub async fn start_background(&self, ctx: Context) {
        if self.background.swap(true, Ordering::SeqCst) {
            return;
        }
        let bot = self.clone();
        tokio::spawn(async move {
            let intents = bot.settings.lock().await.db.intents().unwrap_or_default();
            for intent in intents.iter().filter(|i| i.active) {
                if intent
                    .sleep_until
                    .is_some_and(|t| t <= chrono::Utc::now().timestamp())
                {
                    let mut ended = intent.clone();
                    ended.active = false;
                    let _ = bot.settings.lock().await.db.intent(&ended);
                    continue;
                }
                let guild = GuildId::new(intent.guild);
                let cfg = match bot.settings.lock().await.db.settings(intent.guild) {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                if cfg.radio_mode != RadioMode::Always
                    && humans(
                        &ctx.cache,
                        &ctx.http,
                        guild,
                        ChannelId::new(intent.channel),
                        false,
                    )
                    .await
                    .map(|p| p.is_empty())
                    .unwrap_or(true)
                {
                    continue;
                }
                if let Some(index) = bot
                    .catalog
                    .stations
                    .iter()
                    .position(|s| s.id == intent.station)
                {
                    for attempt in 0..3 {
                        if bot
                            .start_session(
                                &ctx,
                                guild,
                                ChannelId::new(intent.channel),
                                ChannelId::new(intent.text_channel),
                                index,
                                rand::random::<u64>().max(1),
                                Some(&intent.track),
                                intent.sleep_until,
                            )
                            .await
                            .is_ok()
                        {
                            break;
                        }
                        eprintln!(
                            "Music recovery attempt {} failed for guild {}.",
                            attempt + 1,
                            intent.guild
                        );
                        tokio::time::sleep(Duration::from_secs(2)).await;
                    }
                }
            }
            let servers = bot.settings.lock().await.db.servers().unwrap_or_default();
            for guild in servers {
                if !intents.iter().any(|i| i.guild == guild) {
                    let _ = bot.reconcile(&ctx, GuildId::new(guild), true).await;
                }
            }
            let mut last = std::time::Instant::now();
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let elapsed = last.elapsed().as_secs();
                if elapsed == 0 {
                    continue;
                }
                last = std::time::Instant::now();
                let active: Vec<_> = bot
                    .sessions
                    .lock()
                    .await
                    .iter()
                    .map(|(g, s)| (*g, s.clone()))
                    .collect();
                for (guild, s) in active {
                    if let Err(e) = bot.reconcile(&ctx, guild, false).await {
                        eprintln!("Could not reconcile Music: {e}");
                    }
                    if !s.active.load(Ordering::SeqCst) || s.paused.load(Ordering::SeqCst) {
                        continue;
                    }
                    let Some(r) = &s.runtime else {
                        continue;
                    };
                    let generation = s.generation.load(Ordering::SeqCst);
                    let index = s.current.load(Ordering::SeqCst);
                    let enabled = bot
                        .settings
                        .lock()
                        .await
                        .db
                        .settings(guild.get())
                        .map(|s| s.statistics)
                        .unwrap_or(false);
                    if enabled
                        && let Ok(people) =
                            humans(&ctx.cache, &ctx.http, guild, r.channel, true).await
                    {
                        let mut store = bot.settings.lock().await;
                        if !s.active.load(Ordering::SeqCst)
                            || s.paused.load(Ordering::SeqCst)
                            || generation != s.generation.load(Ordering::SeqCst)
                            || index != s.current.load(Ordering::SeqCst)
                            || !store
                                .db
                                .settings(guild.get())
                                .is_ok_and(|settings| settings.statistics)
                        {
                            continue;
                        }
                        let track = &bot.catalog.stations[s.station].tracks[index];
                        if let Err(e) =
                            store
                                .db
                                .record_listening(guild.get(), &track.id, &people, elapsed)
                        {
                            eprintln!("Could not save time-in-call: {e}");
                        }
                    }
                }
            }
        });
    }

    pub async fn authorize(
        &self,
        ctx: &Context,
        guild: GuildId,
        member: &Member,
        text: ChannelId,
        control: bool,
    ) -> Result<ServerSettings, Error> {
        let settings = self.settings.lock().await.db.settings(guild.get())?;
        let manager = self.member_flags(ctx, guild, member)?.0;
        if !self.message_channel_allowed(ctx, guild, &settings, text) {
            return Err("Music is not allowed in this channel.".into());
        }
        if control
            && (member.user.bot
                || !settings.permits_control(
                    manager,
                    &member.roles.iter().map(|r| r.get()).collect::<Vec<_>>(),
                ))
        {
            return Err("Only DJs or server managers can use these playback controls.".into());
        }
        Ok(settings)
    }

    fn message_channel_allowed(
        &self,
        ctx: &Context,
        guild: GuildId,
        settings: &ServerSettings,
        channel: ChannelId,
    ) -> bool {
        let voice = ctx
            .cache
            .guild(guild)
            .and_then(|g| {
                g.channels
                    .get(&channel)
                    .map(|c| c.kind == ChannelType::Voice)
            })
            .unwrap_or(false);
        settings.permits_message_channel(channel.get(), voice)
    }

    pub async fn extra_command(
        &self,
        ctx: &Context,
        command: &CommandInteraction,
    ) -> Result<Option<String>, Error> {
        let g = command.guild_id.ok_or("Use Music in a server.")?;
        let u = command.user.id.get();
        let option = |name: &str| {
            command
                .data
                .options
                .iter()
                .find(|o| o.name == name)
                .map(|o| &o.value)
        };
        let name = command.data.name.as_str();

        let simple = match name {
            "help" => Some(
                "Start with /play. Playback: /station, /pause, /resume, /skip, /stop, /volume, /sleep and /panel. Preferences: /like, /dislike and /library. Managers: /settings, /radio-mode and /skip-settings. Privacy: /profile, /leaderboard and /privacy. Listening to curated stations is free. Some features depend on your Vozen plan. Compare Free, Plus and Premium at https://vozen.org/premium.",
            ),
            "support" | "feedback" => Some(
                "Vozen support and feedback: https://discord.gg/4kYw2WUbNN. No feedback was sent by this command.",
            ),
            _ => None,
        };
        if let Some(text) = simple {
            return Ok(Some(text.into()));
        }
        if name == "invite" {
            let application = ctx
                .http
                .application_id()
                .ok_or("Application unavailable.")?;
            return Ok(Some(format!(
                "https://discord.com/oauth2/authorize?client_id={application}&scope=bot%20applications.commands&permissions=3197952"
            )));
        }
        if name == "settings" {
            return Ok(Some(format!(
                "Open Music settings: https://api.vozen.org/music/{g}"
            )));
        }
        if name == "privacy" {
            let store = self.settings.lock().await;
            let (mut privacy, _) = store.db.privacy(g.get(), u)?;
            if let Some(v) = option("enabled").and_then(|v| v.as_bool()) {
                privacy.enabled = v;
            }
            if let Some(v) = option("visible").and_then(|v| v.as_bool()) {
                privacy.visible = v;
            }
            let erase = option("erase").and_then(|v| v.as_bool()).unwrap_or(false);
            if erase {
                privacy = Privacy::default();
            }
            if !command.data.options.is_empty() {
                store.db.set_privacy(g.get(), u, privacy.clone(), erase)?;
            }
            return Ok(Some(format!(
                "Time-in-call statistics: {}. Leaderboard visibility: {}. Favorites were not changed.",
                privacy.enabled, privacy.visible
            )));
        }
        if name == "profile" {
            let store = self.settings.lock().await;
            let (privacy, seconds) = store.db.privacy(g.get(), u)?;
            return Ok(Some(format!(
                "Time in this server's call while Music was playing and you were not deafened: {} minutes. Collection: {}. Leaderboard: {}. Use /privacy to opt in, hide or erase.",
                seconds / 60,
                privacy.enabled,
                privacy.visible
            )));
        }
        if name == "leaderboard" {
            let store = self.settings.lock().await;
            if !store.db.settings(g.get())?.statistics {
                return Ok(Some("Statistics are disabled for this server.".into()));
            }
            let rows = store.db.leaderboard(g.get())?;
            let lines = rows
                .iter()
                .enumerate()
                .map(|(i, (id, seconds))| format!("{}. <@{}> — {} min", i + 1, id, seconds / 60))
                .collect::<Vec<_>>();
            return Ok(Some(if lines.is_empty() {
                "No visible opted-in profiles yet.".into()
            } else {
                format!(
                    "Time in call while Music was playing (not proof of listening):\n{}",
                    lines.join("\n")
                )
            }));
        }
        if name == "library" {
            let store = self.settings.lock().await;
            if option("clear").and_then(|v| v.as_bool()) == Some(true) {
                store.db.clear_ratings(g.get(), u)?;
            }
            if let Some(id) = option("remove").and_then(|v| v.as_str()) {
                store.db.rate(g.get(), u, id, 0)?;
            }
            let ratings = store.db.ratings(g.get(), &BTreeSet::from([u]))?;
            let page = option("page").and_then(|v| v.as_i64()).unwrap_or(1).max(1) as usize;
            let rows = ratings
                .get(&u)
                .into_iter()
                .flat_map(|r| r.iter())
                .skip((page - 1) * 10)
                .take(10)
                .map(|(id, value)| {
                    let title = self
                        .catalog
                        .stations
                        .iter()
                        .flat_map(|s| &s.tracks)
                        .find(|t| &t.id == id)
                        .map(|t| t.title.as_str())
                        .unwrap_or("Unavailable recording")
                        .chars()
                        .take(65)
                        .collect::<String>();
                    let label = if *value == 1 { "Liked" } else { "Disliked" };
                    format!("{label} {title} · `{id}`")
                })
                .collect::<Vec<_>>();
            return Ok(Some(format!(
                "Your ratings · page {page}\n{}",
                if rows.is_empty() {
                    "No ratings on this page.".into()
                } else {
                    rows.join("\n")
                }
            )));
        }
        if name == "radio-mode" {
            let member = command.member.as_deref().ok_or("Member unavailable.")?;
            if !self.member_flags(ctx, g, member)?.0 {
                return Err("Manage Server is required.".into());
            }
            let mut store = self.settings.lock().await;
            let mut config = store.db.settings(g.get())?;
            if command.data.options.is_empty() {
                return Ok(Some(format!(
                    "Current radio mode: {:?}.",
                    config.radio_mode
                )));
            }
            if let Some(mode) = option("mode").and_then(|v| v.as_str()) {
                config.radio_mode = match mode {
                    "session" => RadioMode::Session,
                    "always" => RadioMode::Always,
                    "auto_join" => RadioMode::AutoJoin,
                    _ => return Err("Invalid radio mode.".into()),
                };
            }
            if let Some(channel) = option("channel").and_then(|v| v.as_channel_id()) {
                config.voice_channel = Some(channel.get());
            }
            {
                let server = ctx.cache.guild(g).ok_or("Server unavailable.")?;
                let roles = server.roles.keys().map(|r| r.get()).collect();
                let voice = server
                    .channels
                    .values()
                    .filter(|c| c.kind == ChannelType::Voice)
                    .map(|c| c.id.get())
                    .collect();
                let text = server
                    .channels
                    .values()
                    .filter(|c| c.kind == ChannelType::Text)
                    .map(|c| c.id.get())
                    .collect();
                let stations = self.catalog.stations.iter().map(|s| s.id.clone()).collect();
                config.validate_resources(g.get(), &roles, &voice, &text, &stations)?;
            }
            let mode = config.radio_mode;
            store.save(g.get(), config)?;
            return Ok(Some(format!("Radio mode saved: {mode:?}.")));
        }
        Ok(None)
    }

    pub async fn selection(&self, session: &Session) -> Result<Option<usize>, Error> {
        match &session.runtime {
            Some(r) => r.next(&self.catalog, session).await,
            None => Ok(vozen_music::next_track(
                session.current.load(Ordering::SeqCst),
                self.catalog.stations[session.station].tracks.len(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn auto_join_notice_reaches_selected_text_channel_without_pings_and_reports_denial() {
        use axum::{Json, Router, http::StatusCode, routing::post};
        let received = Arc::new(Mutex::new(Vec::new()));
        let requests = received.clone();
        let app = Router::new().route(
            "/api/v10/channels/{channel}/messages",
            post(move |axum::extract::Path(channel): axum::extract::Path<u64>, Json(body): Json<serde_json::Value>| {
                let requests = requests.clone();
                async move {
                    requests.lock().await.push((channel, body.clone()));
                    if channel == 43 {
                        return (StatusCode::FORBIDDEN, Json(serde_json::json!({"code":50013,"message":"Missing Permissions"})));
                    }
                    let mut message = Message::default();
                    message.id = MessageId::new(1);
                    message.channel_id = ChannelId::new(channel);
                    message.content = body["content"].as_str().unwrap().to_owned();
                    (StatusCode::OK, Json(serde_json::to_value(message).unwrap()))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let http = serenity::http::HttpBuilder::new("synthetic-token")
            .proxy(format!("http://{address}"))
            .ratelimiter_disabled(true)
            .build();
        let message = "Station selected: @everyone <@123>. Player controls are in <#99>.";
        tokio::time::timeout(
            Duration::from_secs(5),
            announce_auto_join(&http, ChannelId::new(42), message.to_owned()),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(
            tokio::time::timeout(
                Duration::from_secs(5),
                announce_auto_join(&http, ChannelId::new(43), message.to_owned()),
            )
            .await
            .unwrap()
            .is_err()
        );
        let received = received.lock().await;
        assert_eq!(
            received
                .iter()
                .map(|(channel, _)| *channel)
                .collect::<Vec<_>>(),
            vec![42, 43]
        );
        for (_, body) in received.iter() {
            assert_eq!(body["content"], message);
            assert_eq!(body["allowed_mentions"]["parse"], serde_json::json!([]));
        }
        server.abort();
    }

    #[test]
    fn play_and_station_are_throttled_per_user_and_per_guild() {
        let now = std::time::Instant::now();
        let (a, b) = (GuildId::new(1), GuildId::new(2));
        let (alice, bob) = (UserId::new(10), UserId::new(11));
        let mut cooldowns = PlayCooldowns::default();
        cooldowns.begin(a, alice, now).unwrap();
        // Same user anywhere, or anyone in the same guild, must wait.
        assert!(cooldowns.begin(b, alice, now).is_err());
        assert!(cooldowns.begin(a, bob, now).is_err());
        cooldowns.begin(b, bob, now).unwrap();
        let later = now + GUILD_PLAY_COOLDOWN;
        assert!(cooldowns.begin(a, alice, later).is_err());
        cooldowns.begin(a, UserId::new(12), later).unwrap();
        cooldowns
            .begin(GuildId::new(3), alice, now + USER_PLAY_COOLDOWN)
            .unwrap();
        // Expired entries are pruned so the maps stay small.
        cooldowns
            .begin(
                GuildId::new(4),
                UserId::new(13),
                now + Duration::from_secs(60),
            )
            .unwrap();
        assert_eq!((cooldowns.users.len(), cooldowns.guilds.len()), (1, 1));
    }

    #[tokio::test]
    async fn replay_eligibility_uses_only_the_present_listeners_ratings() {
        let mut guild = Guild::default();
        guild.id = GuildId::new(1);
        guild.voice_states.insert(UserId::new(3), serde_json::from_value(serde_json::json!({
            "guild_id":"1","channel_id":"2","user_id":"3","session_id":"fixture",
                "deaf":false,"mute":false,"self_deaf":false,"self_mute":false,"self_video":false,"suppress":false,
            "member":{"user":{"id":"3","username":"Listener","discriminator":"0","avatar":null,"bot":false},"roles":[],"joined_at":null,"deaf":false,"mute":false,"flags":0}
        })).unwrap());
        let cache = Arc::new(Cache::new());
        let mut event: GuildCreateEvent =
            serde_json::from_value(serde_json::to_value(&guild).unwrap()).unwrap();
        cache.update(&mut event);
        let settings = Arc::new(Mutex::new(
            SettingsStore::load(
                std::env::temp_dir().join(format!("vozen-replay-{}.json", rand::random::<u64>())),
            )
            .unwrap(),
        ));
        settings
            .lock()
            .await
            .db
            .rate(1, 3, "recording", -1)
            .unwrap();
        settings.lock().await.db.rate(1, 4, "another", -1).unwrap();
        let runtime = Runtime {
            guild: GuildId::new(1),
            channel: ChannelId::new(2),
            text: ChannelId::new(2),
            cache: cache.clone(),
            http: Arc::new(Http::new("synthetic-token")),
            settings,
            voice: Weak::new(),
        };
        assert!(!runtime.allows_recording("recording").await.unwrap());
        assert!(runtime.allows_recording("another").await.unwrap());
        guild.voice_states.clear();
        let mut event: GuildCreateEvent =
            serde_json::from_value(serde_json::to_value(&guild).unwrap()).unwrap();
        cache.update(&mut event);
        assert!(runtime.allows_recording("recording").await.unwrap());
    }

    #[tokio::test]
    async fn timer_boundaries_cancel_and_replacement_session_are_isolated() {
        let old = Session::new(0, 0, 1, SkipSettings::default(), None);
        let replacement = Session::new(0, 0, 2, SkipSettings::default(), None);
        assert!(set_timer(&old, Some(0), false).await.is_err());
        assert!(set_timer(&old, Some(721), false).await.is_err());
        assert!(set_timer(&old, Some(1), true).await.is_err());
        set_timer(&old, Some(1), false).await.unwrap();
        assert!(old.sleep_until.load(Ordering::SeqCst) > chrono::Utc::now().timestamp() as usize);
        assert_eq!(replacement.sleep_until.load(Ordering::SeqCst), 0);
        set_timer(&old, None, true).await.unwrap();
        assert_eq!(old.sleep_until.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn component_identity_can_be_forwarded_without_changing_actor() {
        let packet = serde_json::json!({"id":"1","application_id":"2","guild_id":"3","channel_id":"4","token":"fixture-only","version":1,"locale":"en-US","entitlements":[],"attachment_size_limit":8388608,"user":{"id":"5","username":"Tester","discriminator":"0","avatar":null},"data":{"id":"1","name":"like","type":1,"options":[{"name":"player-id","type":3,"value":"99"},{"name":"generation","type":4,"value":2}]}});
        let c: CommandInteraction = serde_json::from_value(packet).unwrap();
        assert_eq!(c.user.id.get(), 5);
        assert_eq!(c.guild_id.unwrap().get(), 3);
        assert_eq!(c.data.options[0].value.as_str(), Some("99"));
        assert_eq!(c.data.options[1].value.as_i64(), Some(2));
    }
}
