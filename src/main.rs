mod controls;
mod dashboard;
mod player;

use serenity::{all::*, async_trait};
use songbird::{
    Call, Event, EventContext, EventHandler as AudioEventHandler, SerenityInit, TrackEvent,
    events::EventData,
    input::File,
    tracks::{PlayMode, Track, TrackHandle},
};
use std::{
    collections::HashMap,
    path::Path,
    sync::{
        Arc, Mutex as SyncMutex, Weak,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::Mutex;
use vozen_music::skip::{SettingsStore, SkipSettings, SkipVotes};
use vozen_music::{
    Catalog, Error, check_application_identity, next_after_playback, same_voice_channel,
};

struct PlayerPanel {
    channel: ChannelId,
    message: MessageId,
    update: Mutex<()>,
}

struct Session {
    station: usize,
    active: AtomicBool,
    current: AtomicUsize,
    generation: AtomicUsize,
    consecutive_errors: AtomicUsize,
    handle: SyncMutex<Option<TrackHandle>>,
    control_id: u64,
    panel: Option<Arc<PlayerPanel>>,
    policy: SyncMutex<SkipSettings>,
    votes: SyncMutex<SkipVotes>,
}

impl Session {
    fn new(
        station: usize,
        index: usize,
        control_id: u64,
        policy: SkipSettings,
        panel: Option<Arc<PlayerPanel>>,
    ) -> Self {
        Self {
            station,
            active: AtomicBool::new(true),
            current: AtomicUsize::new(index),
            generation: AtomicUsize::new(0),
            consecutive_errors: AtomicUsize::new(0),
            handle: SyncMutex::new(None),
            control_id,
            panel,
            policy: SyncMutex::new(policy),
            votes: SyncMutex::new(SkipVotes::default()),
        }
    }
}

struct Bot {
    catalog: Arc<Catalog>,
    guild: Option<GuildId>,
    sessions: Mutex<HashMap<GuildId, Arc<Session>>>,
    settings: Mutex<SettingsStore>,
}

#[derive(Clone)]
struct Advance {
    call: Weak<Mutex<Call>>,
    catalog: Arc<Catalog>,
    session: Arc<Session>,
    http: Arc<Http>,
    text_channel: ChannelId,
    generation: usize,
}

fn play_track(call: &mut Call, mut advance: Advance, index: usize) -> Result<(), Error> {
    let track = &advance.catalog.stations[advance.session.station].tracks[index];
    advance.session.current.store(index, Ordering::SeqCst);
    // Invalidate callbacks before replacing the previous track, including its stop event.
    advance.generation = advance.session.generation.fetch_add(1, Ordering::SeqCst) + 1;
    advance
        .session
        .votes
        .lock()
        .map_err(|_| "Skip votes are unavailable.")?
        .reset(advance.generation);
    let mut audio =
        Track::from(File::new(track.file.clone())).volume(10.0_f32.powf(track.gain_db / 20.0));
    // Errored tracks also emit End; register a single callback before playback starts.
    audio.events.add_event(
        EventData::new(Event::Track(TrackEvent::End), advance.clone()),
        Duration::ZERO,
    );
    let handle = call.play_only(audio);
    *advance
        .session
        .handle
        .lock()
        .map_err(|_| "Playback state is unavailable.")? = Some(handle);
    Ok(())
}

#[async_trait]
impl AudioEventHandler for Advance {
    async fn act(&self, ctx: &EventContext<'_>) -> Option<Event> {
        let Some(call) = self.call.upgrade() else {
            self.session.active.store(false, Ordering::SeqCst);
            return Some(Event::Cancel);
        };
        let failed = {
            let mut call = call.lock().await;
            if !self.session.active.load(Ordering::SeqCst)
                || self.generation != self.session.generation.load(Ordering::SeqCst)
            {
                return Some(Event::Cancel);
            }
            let count = self.catalog.stations[self.session.station].tracks.len();
            let errored = matches!(ctx, EventContext::Track(tracks)
                if tracks.iter().any(|(state, _)| matches!(state.playing, PlayMode::Errored(_))));
            let next = next_after_playback(
                self.session.current.load(Ordering::SeqCst),
                count,
                self.session.consecutive_errors.load(Ordering::SeqCst),
                errored,
            );
            if errored {
                eprintln!("An audio track failed; attempting the next station track.");
            }
            let failed = match next {
                Some((index, errors)) => {
                    self.session
                        .consecutive_errors
                        .store(errors, Ordering::SeqCst);
                    play_track(&mut call, self.clone(), index).is_err()
                }
                None => true,
            };
            if failed {
                self.session.active.store(false, Ordering::SeqCst);
                call.stop();
            }
            failed
        };
        player::refresh(&self.http, &self.catalog, &self.session, failed).await;
        if failed {
            let message = CreateMessage::new()
                .content(
                    "The station stopped after repeated audio failures. Use /play to try again.",
                )
                .allowed_mentions(CreateAllowedMentions::new());
            if self
                .text_channel
                .send_message(&self.http, message)
                .await
                .is_err()
            {
                eprintln!("Could not report an audio error to Discord.");
            }
        }
        Some(Event::Cancel)
    }
}

impl Bot {
    fn commands(&self) -> Vec<CreateCommand> {
        let mut station =
            CreateCommandOption::new(CommandOptionType::String, "station", "Choose a station")
                .required(true);
        for item in &self.catalog.stations {
            station = station.add_string_choice(&item.name, &item.id);
        }
        let mut commands = vec![
            CreateCommand::new("play")
                .description("Start a Vozen station")
                .add_option(station.clone()),
            CreateCommand::new("station")
                .description("Switch stations")
                .add_option(station),
        ];
        for (name, description) in [
            ("nowplaying", "Show the current track and credits"),
            ("pause", "Pause the station"),
            ("resume", "Resume the station"),
            ("skip", "Skip or vote to skip the current track"),
            ("stop", "Stop the station and leave the voice channel"),
        ] {
            commands.push(CreateCommand::new(name).description(description));
        }
        commands.push(controls::definition());
        commands
    }

    async fn command(&self, ctx: &Context, command: &CommandInteraction) -> Result<String, Error> {
        let guild_id = command
            .guild_id
            .ok_or("This command can only be used in a server.")?;
        if self.guild.is_some_and(|allowed| guild_id != allowed) {
            return Err("This version is limited to the development server.".into());
        }
        if command.data.name == "skip-settings" {
            return self.configure_skip(ctx, command).await;
        }
        if command.data.name == "skip" {
            let member = command
                .member
                .as_deref()
                .ok_or("Server member information is unavailable.")?;
            return self
                .request_skip(ctx, guild_id, member, command.id.get(), None)
                .await;
        }
        let manager = songbird::get(ctx)
            .await
            .ok_or("The voice connection service is unavailable.")?;
        let member_channel = {
            let guild = ctx
                .cache
                .guild(guild_id)
                .ok_or("The server is still loading. Please try again.")?;
            guild
                .voice_states
                .get(&command.user.id)
                .and_then(|state| state.channel_id)
        };
        let mut sessions = self.sessions.lock().await;
        let existing = manager.get(guild_id);
        let bot_channel = if let Some(call) = &existing {
            call.lock().await.current_channel().map(|id| id.0.get())
        } else {
            None
        };
        if !same_voice_channel(member_channel.map(|id| id.get()), bot_channel) {
            return Err(
                "Join the same voice channel as Vozen Music to control the station.".into(),
            );
        }
        if matches!(command.data.name.as_str(), "play" | "station") {
            let id = command
                .data
                .options
                .iter()
                .find(|o| o.name == "station")
                .and_then(|o| o.value.as_str())
                .ok_or("Choose a station.")?;
            let station_index = self
                .catalog
                .stations
                .iter()
                .position(|s| s.id == id)
                .ok_or("Unknown station.")?;
            let channel_id = member_channel.ok_or("Join a voice channel first.")?;
            {
                let guild = ctx
                    .cache
                    .guild(guild_id)
                    .ok_or("The server is unavailable.")?;
                let channel = guild
                    .channels
                    .get(&channel_id)
                    .ok_or("The voice channel is unavailable.")?;
                if channel.kind != ChannelType::Voice {
                    return Err("This version only supports standard voice channels.".into());
                }
                #[allow(deprecated)]
                let permissions = channel
                    .permissions_for_user(&ctx.cache, ctx.cache.current_user().id)
                    .map_err(|_| "Could not verify the bot's channel permissions.")?;
                if !permissions
                    .contains(Permissions::VIEW_CHANNEL | Permissions::CONNECT | Permissions::SPEAK)
                {
                    return Err(
                        "The bot needs View Channel, Connect, and Speak permissions.".into(),
                    );
                }
            }
            let call = match tokio::time::timeout(
                Duration::from_secs(15),
                manager.join(guild_id, channel_id),
            )
            .await
            {
                Ok(Ok(call)) => call,
                _ => {
                    let _ = tokio::time::timeout(Duration::from_secs(5), manager.remove(guild_id))
                        .await;
                    return Err(
                        "Could not join the voice channel. Check permissions and try again.".into(),
                    );
                }
            };
            let policy = self.settings.lock().await.get(guild_id.get());
            let mut session = Session::new(station_index, 0, command.id.get(), policy, None);
            let message = match command
                .channel_id
                .send_message(&ctx.http, player::message(&self.catalog, &session)?)
                .await
            {
                Ok(message) => message,
                Err(_) => {
                    if !sessions.contains_key(&guild_id) {
                        let _ =
                            tokio::time::timeout(Duration::from_secs(5), manager.remove(guild_id))
                                .await;
                    }
                    return Err("Could not post the player. Check View Channel and Send Messages permissions.".into());
                }
            };
            session.panel = Some(Arc::new(PlayerPanel {
                channel: command.channel_id,
                message: message.id,
                update: Mutex::new(()),
            }));
            let session = Arc::new(session);
            let mut locked = call.lock().await;
            let old = sessions.remove(&guild_id);
            if let Some(old) = &old {
                old.active.store(false, Ordering::SeqCst);
            }
            locked.stop();
            let advance = Advance {
                call: Arc::downgrade(&call),
                catalog: self.catalog.clone(),
                session: session.clone(),
                http: ctx.http.clone(),
                text_channel: command.channel_id,
                generation: 0,
            };
            play_track(&mut locked, advance, 0)?;
            sessions.insert(guild_id, session);
            drop(locked);
            drop(sessions);
            if let Some(old) = old {
                player::refresh(&ctx.http, &self.catalog, &old, true).await;
            }
            return Ok(format!(
                "Station selected: {}.",
                self.catalog.stations[station_index].name
            ));
        }
        let session = sessions
            .get(&guild_id)
            .ok_or("No station is active. Use /play to start one.")?;
        let call = existing.ok_or("The bot is no longer connected. Use /play to start again.")?;
        let mut locked = call.lock().await;
        if command.data.name == "stop" {
            let stopped = session.clone();
            session.active.store(false, Ordering::SeqCst);
            locked.stop();
            sessions.remove(&guild_id);
            drop(locked);
            tokio::time::timeout(Duration::from_secs(5), manager.remove(guild_id))
                .await
                .map_err(|_| "Playback stopped, but leaving the voice channel timed out.")?
                .map_err(|_| "Playback stopped, but the bot could not leave the voice channel.")?;
            drop(sessions);
            player::refresh(&ctx.http, &self.catalog, &stopped, true).await;
            return Ok("Station stopped.".into());
        }
        if !session.active.load(Ordering::SeqCst) {
            return Err("The station is stopped. Use /play to start again.".into());
        }
        let station = &self.catalog.stations[session.station];
        let current = session.current.load(Ordering::SeqCst);
        if command.data.name == "nowplaying" {
            let track = &station.tracks[current];
            return Ok(format!(
                "{}\n{} — {}\nLicense: {}\nSource: {}",
                station.name, track.title, track.artist, track.license, track.source_url
            ));
        }
        let handle = session
            .handle
            .lock()
            .map_err(|_| "Audio state is unavailable.")?
            .clone()
            .ok_or("No track is currently playing.")?;
        match command.data.name.as_str() {
            "pause" => handle.pause().map_err(|_| "Could not pause playback.")?,
            "resume" => handle.play().map_err(|_| "Could not resume playback.")?,
            _ => return Err("Unknown command.".into()),
        }
        Ok("Playback control applied.".into())
    }
}

#[async_trait]
impl EventHandler for Bot {
    async fn ready(&self, _: Context, _: Ready) {
        println!("Vozen Music connected to Discord.");
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        let interaction = match interaction {
            Interaction::Component(button) if button.data.custom_id.starts_with("vozen:skip:") => {
                let response = CreateInteractionResponseMessage::new()
                    .ephemeral(true)
                    .allowed_mentions(CreateAllowedMentions::new());
                if button
                    .create_response(&ctx.http, CreateInteractionResponse::Defer(response))
                    .await
                    .is_err()
                {
                    return;
                }
                let result = async {
                    let guild = button
                        .guild_id
                        .ok_or("This control can only be used in a server.")?;
                    let member = button
                        .member
                        .as_ref()
                        .ok_or("Server member information is unavailable.")?;
                    let target = player::target(&button.data.custom_id)?;
                    self.request_skip(&ctx, guild, member, button.id.get(), Some(target))
                        .await
                }
                .await;
                let content = result.unwrap_or_else(|error| error.to_string());
                if button
                    .edit_response(
                        &ctx.http,
                        EditInteractionResponse::new()
                            .content(content)
                            .allowed_mentions(CreateAllowedMentions::new()),
                    )
                    .await
                    .is_err()
                {
                    eprintln!("Could not respond to a player control.");
                }
                return;
            }
            other => other,
        };
        let Interaction::Command(command) = interaction else {
            return;
        };
        let response = CreateInteractionResponseMessage::new()
            .ephemeral(true)
            .allowed_mentions(CreateAllowedMentions::new());
        if command
            .create_response(&ctx.http, CreateInteractionResponse::Defer(response))
            .await
            .is_err()
        {
            eprintln!("Could not acknowledge an interaction.");
            return;
        }
        let result = self.command(&ctx, &command).await;
        let success = result.is_ok();
        let content = match result {
            Ok(message) => message,
            Err(error) => error.to_string(),
        };
        let mut components = vec![];
        if success
            && command.data.name == "nowplaying"
            && let Some(guild) = command.guild_id
            && let Some(session) = self.sessions.lock().await.get(&guild)
            && session.active.load(Ordering::SeqCst)
        {
            components =
                player::controls(session, session.generation.load(Ordering::SeqCst), false)
                    .unwrap_or_default();
        }
        if command
            .edit_response(
                &ctx.http,
                EditInteractionResponse::new()
                    .content(content)
                    .components(components)
                    .allowed_mentions(CreateAllowedMentions::new()),
            )
            .await
            .is_err()
        {
            eprintln!("Could not respond to an interaction.");
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if !args.is_empty() && args != ["--check"] {
        return Err("Usage: vozen-music [--check], from the catalog directory.".into());
    }
    let catalog = Arc::new(Catalog::load(Path::new("."))?);
    if args == ["--check"] {
        let tracks: usize = catalog.stations.iter().map(|s| s.tracks.len()).sum();
        println!(
            "Catalog structure is valid: {} station(s), {tracks} track(s).",
            catalog.stations.len()
        );
        catalog.require_audio()?;
        return Ok(());
    }
    catalog.require_audio()?;
    let token = std::env::var("DISCORD_TOKEN")
        .map_err(|_| "Missing DISCORD_TOKEN for the Music application.")?;
    let application_id = std::env::var("DISCORD_APPLICATION_ID")
        .map_err(|_| "Missing DISCORD_APPLICATION_ID for the Music application.")?
        .parse::<u64>()
        .ok()
        .filter(|id| *id > 0)
        .ok_or("Invalid DISCORD_APPLICATION_ID.")?;
    let guild = std::env::var("DISCORD_GUILD_ID")
        .ok()
        .filter(|value| !value.is_empty())
        .map(|value| {
            value
                .parse::<u64>()
                .ok()
                .filter(|id| *id > 0)
                .map(GuildId::new)
                .ok_or("Invalid DISCORD_GUILD_ID.")
        })
        .transpose()?;
    let bot = Arc::new(Bot {
        catalog,
        guild,
        sessions: Mutex::new(HashMap::new()),
        settings: Mutex::new(SettingsStore::load(
            Path::new("data/skip-settings.json").to_path_buf(),
        )?),
    });
    let commands = bot.commands();
    let intents = GatewayIntents::GUILDS | GatewayIntents::GUILD_VOICE_STATES;
    let mut client = Client::builder(token, intents)
        .event_handler_arc(bot.clone())
        .register_songbird()
        .await?;
    let application = client.http.get_current_application_info().await?;
    check_application_identity(application_id, application.id.get())?;
    client.http.set_application_id(application.id);
    if let Some(guild) = guild {
        guild.set_commands(&client.http, commands).await?;
    } else {
        Command::set_global_commands(&client.http, commands).await?;
    }
    let mut dashboard = dashboard::start(bot, client.cache.clone(), client.http.clone()).await?;
    let shards = client.shard_manager.clone();
    #[cfg(unix)]
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    // Ctrl-C closes the gateway; audio tasks end with the runtime.
    tokio::spawn(async move {
        #[cfg(unix)]
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {},
            _ = terminate.recv() => {},
        }
        #[cfg(not(unix))]
        let _ = tokio::signal::ctrl_c().await;
        shards.shutdown_all().await;
    });
    tokio::select! {
        result = client.start() => { result?; },
        result = async {
            match &mut dashboard {
                Some(task) => task.await.map_err(|_| "Dashboard task stopped unexpectedly.")?,
                None => std::future::pending().await,
            }
        } => {
            client.shard_manager.shutdown_all().await;
            result?;
            return Err("Dashboard stopped unexpectedly.".into());
        }
    }
    if let Some(task) = dashboard {
        task.abort();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_commands_expose_english_names_descriptions_and_station_option() {
        let catalog = serde_json::from_str(include_str!("../catalog.json")).unwrap();
        let bot = Bot {
            catalog: Arc::new(catalog),
            guild: Some(GuildId::new(1)),
            sessions: Mutex::new(HashMap::new()),
            settings: Mutex::new(
                SettingsStore::load(std::env::temp_dir().join(format!(
                        "vozen-unused-{}.json",
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .as_nanos()
                    )))
                .unwrap(),
            ),
        };
        let commands = serde_json::to_value(bot.commands()).unwrap();
        let commands = commands.as_array().unwrap();
        let names: Vec<_> = commands
            .iter()
            .map(|c| c["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "play",
                "station",
                "nowplaying",
                "pause",
                "resume",
                "skip",
                "stop",
                "skip-settings"
            ]
        );
        for command in commands {
            let description = command["description"].as_str().unwrap();
            assert!(!description.is_empty() && description.is_ascii());
            // Default English must not depend on the Discord user's locale.
            assert_eq!(command["description_localizations"], serde_json::json!({}));
            if matches!(command["name"].as_str(), Some("play" | "station")) {
                let option = &command["options"][0];
                assert_eq!(option["name"], "station");
                assert_eq!(option["description"], "Choose a station");
                assert_eq!(option["required"], true);
                assert!(!option["choices"].as_array().unwrap().is_empty());
                assert_eq!(
                    option["choices"].as_array().unwrap().len(),
                    bot.catalog.stations.len()
                );
            }
        }
    }

    fn advance(call: Weak<Mutex<Call>>, active: bool) -> Advance {
        Advance {
            call,
            catalog: Arc::new(Catalog { stations: vec![] }),
            session: {
                let session = Arc::new(Session::new(0, 0, 1, SkipSettings::default(), None));
                session.active.store(active, Ordering::SeqCst);
                session
            },
            http: Arc::new(Http::new("synthetic-test-token")),
            text_channel: ChannelId::new(1),
            generation: 0,
        }
    }

    #[tokio::test]
    async fn stale_end_event_after_stop_or_switch_cannot_restart_audio() {
        let call = Arc::new(Mutex::new(Call::standalone(
            GuildId::new(1),
            UserId::new(2),
        )));
        let handler = advance(Arc::downgrade(&call), false);
        assert!(matches!(
            handler.act(&EventContext::Track(&[])).await,
            Some(Event::Cancel)
        ));
        assert!(handler.session.handle.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn stale_track_generation_cannot_advance_the_next_track() {
        let call = Arc::new(Mutex::new(Call::standalone(
            GuildId::new(1),
            UserId::new(2),
        )));
        let handler = advance(Arc::downgrade(&call), true);
        handler.session.generation.store(1, Ordering::SeqCst);
        assert!(matches!(
            handler.act(&EventContext::Track(&[])).await,
            Some(Event::Cancel)
        ));
        assert!(handler.session.handle.lock().unwrap().is_none());
        assert!(handler.session.active.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn a_decode_failure_advances_once() {
        let call = Arc::new(Mutex::new(Call::standalone(
            GuildId::new(1),
            UserId::new(2),
        )));
        let mut handler = advance(Arc::downgrade(&call), true);
        handler.catalog = Arc::new(Catalog::load(Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap());
        let old_handle = call
            .lock()
            .await
            .play_only_input(File::new(handler.catalog.stations[0].tracks[0].file.clone()).into());
        let state = songbird::tracks::TrackState {
            playing: PlayMode::Errored(songbird::tracks::PlayError::Decode(Arc::new(
                symphonia::core::errors::Error::DecodeError("synthetic decode failure"),
            ))),
            ..Default::default()
        };
        let tracks = [(&state, &old_handle)];
        let context = EventContext::Track(&tracks);
        assert!(matches!(handler.act(&context).await, Some(Event::Cancel)));
        assert_eq!(handler.session.current.load(Ordering::SeqCst), 1);
        assert_eq!(handler.session.consecutive_errors.load(Ordering::SeqCst), 1);
        assert!(handler.session.active.load(Ordering::SeqCst));
        // A second callback for the failed track must not skip a healthy track.
        assert!(matches!(handler.act(&context).await, Some(Event::Cancel)));
        assert_eq!(handler.session.current.load(Ordering::SeqCst), 1);
        handler.session.active.store(false, Ordering::SeqCst);
        call.lock().await.stop();
    }

    #[tokio::test]
    async fn dropped_voice_call_deactivates_station_without_network() {
        let handler = advance(Weak::new(), true);
        assert!(matches!(
            handler.act(&EventContext::Track(&[])).await,
            Some(Event::Cancel)
        ));
        assert!(!handler.session.active.load(Ordering::SeqCst));
    }
}
