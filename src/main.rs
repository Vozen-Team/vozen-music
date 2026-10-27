use serenity::{all::*, async_trait};
use songbird::{
    Call, Event, EventContext, EventHandler as AudioEventHandler, SerenityInit, TrackEvent,
    input::File, tracks::TrackHandle,
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
use vozen_music::{Catalog, Error, check_application_identity, next_track, same_voice_channel};

struct Session {
    station: usize,
    active: AtomicBool,
    current: AtomicUsize,
    handle: SyncMutex<Option<TrackHandle>>,
}

struct Bot {
    catalog: Arc<Catalog>,
    guild: GuildId,
    sessions: Mutex<HashMap<GuildId, Arc<Session>>>,
}

#[derive(Clone)]
struct Advance {
    call: Weak<Mutex<Call>>,
    catalog: Arc<Catalog>,
    session: Arc<Session>,
    http: Arc<Http>,
    text_channel: ChannelId,
    failed: bool,
}

fn play_track(call: &mut Call, advance: Advance, index: usize) -> Result<(), Error> {
    let track = &advance.catalog.stations[advance.session.station].tracks[index];
    let handle = call.play_only_input(File::new(track.file.clone()).into());
    let mut on_error = advance.clone();
    on_error.failed = true;
    let result = handle
        .add_event(Event::Track(TrackEvent::End), advance.clone())
        .and_then(|_| handle.add_event(Event::Track(TrackEvent::Error), on_error));
    if result.is_err() {
        advance.session.active.store(false, Ordering::SeqCst);
        call.stop();
        return Err("Could not set up track controls.".into());
    }
    advance.session.current.store(index, Ordering::SeqCst);
    *advance
        .session
        .handle
        .lock()
        .map_err(|_| "Playback state is unavailable.")? = Some(handle);
    Ok(())
}

#[async_trait]
impl AudioEventHandler for Advance {
    async fn act(&self, _: &EventContext<'_>) -> Option<Event> {
        let Some(call) = self.call.upgrade() else {
            self.session.active.store(false, Ordering::SeqCst);
            return Some(Event::Cancel);
        };
        let failed = {
            let mut call = call.lock().await;
            if !self.session.active.load(Ordering::SeqCst) {
                return Some(Event::Cancel);
            }
            let count = self.catalog.stations[self.session.station].tracks.len();
            let next = next_track(self.session.current.load(Ordering::SeqCst), count);
            let failed = self.failed
                || match next {
                    Some(index) => play_track(&mut call, self.clone(), index).is_err(),
                    None => true,
                };
            if failed {
                self.session.active.store(false, Ordering::SeqCst);
                call.stop();
            }
            failed
        };
        if failed {
            let message = CreateMessage::new()
                .content("The station stopped because of an audio error. Use /play to try again.")
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
            ("skip", "Skip to the next track"),
            ("stop", "Stop the station and leave the voice channel"),
        ] {
            commands.push(CreateCommand::new(name).description(description));
        }
        commands
    }

    async fn command(&self, ctx: &Context, command: &CommandInteraction) -> Result<String, Error> {
        let guild_id = command
            .guild_id
            .ok_or("This command can only be used in a server.")?;
        if guild_id != self.guild {
            return Err("This version is limited to the development server.".into());
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
            let mut locked = call.lock().await;
            if let Some(old) = sessions.remove(&guild_id) {
                old.active.store(false, Ordering::SeqCst);
            }
            locked.stop();
            let session = Arc::new(Session {
                station: station_index,
                active: AtomicBool::new(true),
                current: AtomicUsize::new(0),
                handle: SyncMutex::new(None),
            });
            let advance = Advance {
                call: Arc::downgrade(&call),
                catalog: self.catalog.clone(),
                session: session.clone(),
                http: ctx.http.clone(),
                text_channel: command.channel_id,
                failed: false,
            };
            play_track(&mut locked, advance, 0)?;
            sessions.insert(guild_id, session);
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
            session.active.store(false, Ordering::SeqCst);
            locked.stop();
            sessions.remove(&guild_id);
            drop(locked);
            tokio::time::timeout(Duration::from_secs(5), manager.remove(guild_id))
                .await
                .map_err(|_| "Playback stopped, but leaving the voice channel timed out.")?
                .map_err(|_| "Playback stopped, but the bot could not leave the voice channel.")?;
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
            "skip" => {
                // Replacing the session invalidates old callbacks, including the End caused by skip.
                session.active.store(false, Ordering::SeqCst);
                let next =
                    next_track(current, station.tracks.len()).ok_or("The station is empty.")?;
                let replacement = Arc::new(Session {
                    station: session.station,
                    active: AtomicBool::new(true),
                    current: AtomicUsize::new(next),
                    handle: SyncMutex::new(None),
                });
                let advance = Advance {
                    call: Arc::downgrade(&call),
                    catalog: self.catalog.clone(),
                    session: replacement.clone(),
                    http: ctx.http.clone(),
                    text_channel: command.channel_id,
                    failed: false,
                };
                play_track(&mut locked, advance, next)?;
                sessions.insert(guild_id, replacement);
            }
            _ => return Err("Unknown command.".into()),
        }
        Ok("Playback control applied.".into())
    }
}

#[async_trait]
impl EventHandler for Bot {
    async fn ready(&self, _: Context, _: Ready) {
        println!("Vozen Music connected to the development server.");
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
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
        let content = match self.command(&ctx, &command).await {
            Ok(message) => message,
            Err(error) => error.to_string(),
        };
        if command
            .edit_response(
                &ctx.http,
                EditInteractionResponse::new()
                    .content(content)
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
    let guild = GuildId::new(
        std::env::var("DISCORD_GUILD_ID")
            .map_err(|_| "Missing development DISCORD_GUILD_ID.")?
            .parse::<u64>()
            .ok()
            .filter(|id| *id > 0)
            .ok_or("Invalid DISCORD_GUILD_ID.")?,
    );
    let bot = Bot {
        catalog,
        guild,
        sessions: Mutex::new(HashMap::new()),
    };
    let commands = bot.commands();
    let intents = GatewayIntents::GUILDS | GatewayIntents::GUILD_VOICE_STATES;
    let mut client = Client::builder(token, intents)
        .event_handler(bot)
        .register_songbird()
        .await?;
    let application = client.http.get_current_application_info().await?;
    check_application_identity(application_id, application.id.get())?;
    client.http.set_application_id(application.id);
    guild.set_commands(&client.http, commands).await?;
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
    client.start().await?;
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
            guild: GuildId::new(1),
            sessions: Mutex::new(HashMap::new()),
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
                "stop"
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
            }
        }
    }

    fn advance(call: Weak<Mutex<Call>>, active: bool) -> Advance {
        Advance {
            call,
            catalog: Arc::new(Catalog { stations: vec![] }),
            session: Arc::new(Session {
                station: 0,
                active: AtomicBool::new(active),
                current: AtomicUsize::new(0),
                handle: SyncMutex::new(None),
            }),
            http: Arc::new(Http::new("synthetic-test-token")),
            text_channel: ChannelId::new(1),
            failed: false,
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
    async fn dropped_voice_call_deactivates_station_without_network() {
        let handler = advance(Weak::new(), true);
        assert!(matches!(
            handler.act(&EventContext::Track(&[])).await,
            Some(Event::Cancel)
        ));
        assert!(!handler.session.active.load(Ordering::SeqCst));
    }
}
