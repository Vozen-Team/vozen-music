mod controls;
mod dashboard;
mod player;
mod player_artwork;
mod runtime;

use serenity::{all::*, async_trait};
use songbird::{
    Call, Event, EventContext, EventHandler as AudioEventHandler, SerenityInit, TrackEvent,
    events::EventData,
    input::File,
    tracks::{PlayMode, Track, TrackHandle},
};
use std::{
    collections::{HashMap, VecDeque},
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
    update: Mutex<Option<player::Artwork>>,
}

struct Session {
    runtime: Option<Arc<runtime::Runtime>>,
    volume: AtomicUsize,
    turn: AtomicUsize,
    recovery_attempts: AtomicUsize,
    sleep_until: AtomicUsize,
    paused: AtomicBool,
    looping: AtomicBool,
    history: SyncMutex<VecDeque<usize>>,
    tips: AtomicBool,
    station: usize,
    active: AtomicBool,
    current: AtomicUsize,
    generation: AtomicUsize,
    consecutive_errors: AtomicUsize,
    handle: SyncMutex<Option<TrackHandle>>,
    control_id: u64,
    panel: SyncMutex<Option<Arc<PlayerPanel>>>,
    policy: SyncMutex<SkipSettings>,
    votes: SyncMutex<SkipVotes>,
    previous_votes: SyncMutex<SkipVotes>,
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
            runtime: None,
            volume: AtomicUsize::new(100),
            turn: AtomicUsize::new(0),
            recovery_attempts: AtomicUsize::new(0),
            sleep_until: AtomicUsize::new(0),
            paused: AtomicBool::new(false),
            looping: AtomicBool::new(false),
            history: SyncMutex::new(VecDeque::new()),
            tips: AtomicBool::new(false),
            station,
            active: AtomicBool::new(true),
            current: AtomicUsize::new(index),
            generation: AtomicUsize::new(0),
            consecutive_errors: AtomicUsize::new(0),
            handle: SyncMutex::new(None),
            control_id,
            panel: SyncMutex::new(panel),
            policy: SyncMutex::new(policy),
            votes: SyncMutex::new(SkipVotes::default()),
            previous_votes: SyncMutex::new(SkipVotes::default()),
        }
    }

    fn for_track_change(
        &self,
        index: usize,
        control_id: u64,
        policy: SkipSettings,
        previous: bool,
    ) -> Result<Self, Error> {
        let mut history = self
            .history
            .lock()
            .map_err(|_| "Playback history is unavailable.")?
            .clone();
        if previous && history.pop_back() != Some(index) {
            return Err("The previous recording changed. Please try again.".into());
        }
        let mut replacement = Self::new(
            self.station,
            if previous {
                index
            } else {
                self.current.load(Ordering::SeqCst)
            },
            control_id,
            policy,
            self.panel.lock().map_err(|_| "Panel unavailable.")?.clone(),
        );
        replacement.runtime = self.runtime.clone();
        replacement.history = SyncMutex::new(history);
        replacement
            .looping
            .store(self.looping.load(Ordering::SeqCst), Ordering::SeqCst);
        replacement
            .tips
            .store(self.tips.load(Ordering::SeqCst), Ordering::SeqCst);
        replacement
            .volume
            .store(self.volume.load(Ordering::SeqCst), Ordering::SeqCst);
        replacement
            .sleep_until
            .store(self.sleep_until.load(Ordering::SeqCst), Ordering::SeqCst);
        replacement
            .turn
            .store(self.turn.load(Ordering::SeqCst), Ordering::SeqCst);
        Ok(replacement)
    }

    fn votes_for(&self, previous: bool) -> &SyncMutex<SkipVotes> {
        if previous {
            &self.previous_votes
        } else {
            &self.votes
        }
    }

    fn reset_votes(&self, generation: usize) -> Result<(), Error> {
        for previous in [false, true] {
            self.votes_for(previous)
                .lock()
                .map_err(|_| "Skip votes are unavailable.")?
                .reset(generation);
        }
        Ok(())
    }

    fn set_track(&self, index: usize) -> Result<(), Error> {
        let current = self.current.load(Ordering::SeqCst);
        if current != index {
            let mut history = self
                .history
                .lock()
                .map_err(|_| "Playback history is unavailable.")?;
            if history.len() == 100 {
                history.pop_front();
            }
            history.push_back(current);
        }
        self.current.store(index, Ordering::SeqCst);
        Ok(())
    }
}

#[derive(Clone)]
struct Bot {
    catalog: Arc<Catalog>,
    background: Arc<AtomicBool>,
    guild: Option<GuildId>,
    sessions: Arc<Mutex<HashMap<GuildId, Arc<Session>>>>,
    settings: Arc<Mutex<SettingsStore>>,
    play_cooldowns: Arc<SyncMutex<runtime::PlayCooldowns>>,
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
    advance.session.set_track(index)?;
    advance.session.turn.fetch_add(1, Ordering::SeqCst);
    // Invalidate callbacks before replacing the previous track, including its stop event.
    advance.generation = advance.session.generation.fetch_add(1, Ordering::SeqCst) + 1;
    advance.session.reset_votes(advance.generation)?;
    let mut audio = Track::from(File::new(track.file.clone())).volume(
        10.0_f32.powf(track.gain_db / 20.0) * advance.session.volume.load(Ordering::SeqCst) as f32
            / 100.0,
    );
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
            if let Err(error) = player::remove(&self.http, &self.session).await {
                eprintln!("Could not delete the disconnected player: {error}");
            }
            return Some(Event::Cancel);
        };
        if !self.session.active.load(Ordering::SeqCst)
            || self.generation != self.session.generation.load(Ordering::SeqCst)
        {
            return Some(Event::Cancel);
        }
        let errored = matches!(ctx, EventContext::Track(tracks)
            if tracks.iter().any(|(state, _)| matches!(state.playing, PlayMode::Errored(_))));
        let preferred = if self.session.looping.load(Ordering::SeqCst) && !errored {
            let index = self.session.current.load(Ordering::SeqCst);
            match &self.session.runtime {
                Some(r) => r
                    .allows_recording(&self.catalog.stations[self.session.station].tracks[index].id)
                    .await
                    .ok()
                    .filter(|allowed| *allowed)
                    .map(|_| index),
                None => Some(index),
            }
        } else if let Some(r) = &self.session.runtime {
            r.next(&self.catalog, &self.session).await.unwrap_or(None)
        } else {
            Some(
                (self.session.current.load(Ordering::SeqCst) + 1)
                    % self
                        .catalog
                        .stations
                        .get(self.session.station)
                        .map(|s| s.tracks.len())
                        .unwrap_or(1)
                        .max(1),
            )
        };
        let failed = {
            let mut call = call.lock().await;
            if !self.session.active.load(Ordering::SeqCst)
                || self.generation != self.session.generation.load(Ordering::SeqCst)
            {
                return Some(Event::Cancel);
            }
            let count = self.catalog.stations[self.session.station].tracks.len();
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
                Some((_index, errors)) => {
                    let index = if let Some(index) = preferred {
                        index
                    } else {
                        self.session.paused.store(true, Ordering::SeqCst);
                        call.stop();
                        drop(call);
                        let _=self.text_channel.say(&self.http,"Playback paused: no recordings match the listeners preferences. Change ratings and use /play.").await;
                        return Some(Event::Cancel);
                    };
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
        if let Some(r) = &self.session.runtime
            && let Err(e) = r.persist(&self.catalog, &self.session, !failed).await
        {
            eprintln!("Could not persist playback: {e}");
        }
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
                .required(false);
        for item in &self.catalog.stations {
            station = station.add_string_choice(&item.name, &item.id);
        }
        let mut commands = vec![
            CreateCommand::new("play")
                .description("Start a Vozen station")
                .add_option(station.clone()),
            CreateCommand::new("station")
                .description("Switch stations")
                .add_option(station.required(true)),
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
        commands.extend(runtime::definitions());
        commands
    }

    async fn command(&self, ctx: &Context, command: &CommandInteraction) -> Result<String, Error> {
        let guild_id = command
            .guild_id
            .ok_or("This command can only be used in a server.")?;
        if self.guild.is_some_and(|allowed| guild_id != allowed) {
            return Err("This version is limited to the development server.".into());
        }
        if let Some(reply) = self.extra_command(ctx, command).await? {
            return Ok(reply);
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
                .request_skip(
                    ctx,
                    guild_id,
                    member,
                    command.channel_id,
                    command.id.get(),
                    None,
                )
                .await;
        }
        let member = command
            .member
            .as_deref()
            .ok_or("Member information is unavailable.")?;
        let config = self
            .authorize(
                ctx,
                guild_id,
                member,
                command.channel_id,
                !matches!(
                    command.data.name.as_str(),
                    "like" | "dislike" | "nowplaying"
                ),
            )
            .await?;
        if matches!(command.data.name.as_str(), "play" | "station") {
            // Checked before the shared session map is locked for the voice join.
            self.play_cooldowns
                .lock()
                .map_err(|_| "Music is busy. Please try again.")?
                .begin(guild_id, command.user.id, std::time::Instant::now())?;
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
                .unwrap_or(&config.default_station);
            let station_index = self
                .catalog
                .stations
                .iter()
                .position(|s| s.id == id)
                .ok_or("Unknown station.")?;
            let channel_id = member_channel.ok_or("Join a voice channel first.")?;
            drop(sessions);
            return self
                .start_session(
                    ctx,
                    guild_id,
                    channel_id,
                    command.channel_id,
                    station_index,
                    command.id.get(),
                    None,
                    None,
                )
                .await;
        }
        let session = sessions
            .get(&guild_id)
            .ok_or("No station is active. Use /play to start one.")?;
        let call = existing.ok_or("The bot is no longer connected. Use /play to start again.")?;
        let mut locked = call.lock().await;
        let expected_id = command
            .data
            .options
            .iter()
            .find(|o| o.name == "player-id")
            .and_then(|o| o.value.as_str());
        let expected_generation = command
            .data
            .options
            .iter()
            .find(|o| o.name == "generation")
            .and_then(|o| o.value.as_i64());
        if expected_id.is_some_and(|id| id != session.control_id.to_string())
            || expected_generation
                .is_some_and(|v| v != session.generation.load(Ordering::SeqCst) as i64)
        {
            return Err("This player is outdated. Use /panel.".into());
        }
        if matches!(command.data.name.as_str(), "like" | "dislike") {
            let track = &self.catalog.stations[session.station].tracks
                [session.current.load(Ordering::SeqCst)];
            if command.data.name == "like" && expected_id.is_some() {
                let liked = self.settings.lock().await.db.toggle_like(
                    guild_id.get(),
                    command.user.id.get(),
                    &track.id,
                )?;
                return Ok(if liked {
                    "❤️ Added this recording to your likes."
                } else {
                    "♡ Removed this recording from your likes."
                }
                .into());
            }
            self.settings.lock().await.db.rate(
                guild_id.get(),
                command.user.id.get(),
                &track.id,
                if command.data.name == "like" { 1 } else { -1 },
            )?;
            return Ok("Your rating was saved for this server.".into());
        }
        if matches!(
            command.data.name.as_str(),
            "volume" | "volume-down" | "volume-up"
        ) {
            let level =
                player::button_volume(&command.data.name, session.volume.load(Ordering::SeqCst))
                    .or_else(|| {
                        command
                            .data
                            .options
                            .iter()
                            .find(|o| o.name == "level")
                            .and_then(|o| o.value.as_i64())
                    });
            if let Some(level) = level {
                if !(0..=100).contains(&level) {
                    return Err("Volume must be between 0 and 100.".into());
                }
                let mut store = self.settings.lock().await;
                let mut cfg = store.db.settings(guild_id.get())?;
                cfg.volume = level as u8;
                store.save(guild_id.get(), cfg)?;
                let track = &self.catalog.stations[session.station].tracks
                    [session.current.load(Ordering::SeqCst)];
                if let Some(handle) = session
                    .handle
                    .lock()
                    .map_err(|_| "Audio unavailable.")?
                    .as_ref()
                {
                    handle
                        .set_volume(10.0_f32.powf(track.gain_db / 20.0) * level as f32 / 100.0)?;
                }
                session.volume.store(level as usize, Ordering::SeqCst);
            }
            let response = format!("Volume: {}%.", session.volume.load(Ordering::SeqCst));
            let refreshed = session.clone();
            drop(locked);
            drop(sessions);
            if level.is_some() {
                player::refresh(&ctx.http, &self.catalog, &refreshed, false).await;
            }
            return Ok(response);
        }
        if command.data.name == "sleep" {
            let cancel = command
                .data
                .options
                .iter()
                .any(|o| o.name == "cancel" && o.value.as_bool() == Some(true));
            let minutes = command
                .data
                .options
                .iter()
                .find(|o| o.name == "minutes")
                .and_then(|o| o.value.as_i64());
            let deadline = runtime::set_timer(session, minutes, cancel)
                .await?
                .ok_or("No station is active. Use /play to start one.")?;
            return Ok(if deadline == 0 {
                "No sleep timer is active.".into()
            } else {
                format!("Playback stops <t:{deadline}:R>.")
            });
        }
        if command.data.name == "panel" {
            let panel = player::post(ctx, &self.catalog, session).await?;
            *session.panel.lock().map_err(|_| "Panel unavailable.")? = Some(panel);
            return Ok("Player panel posted in voice chat.".into());
        }
        if command.data.name == "stop" {
            let stopped = session.clone();
            let persistence = if let Some(r) = &session.runtime {
                r.persist(&self.catalog, session, false).await
            } else {
                Ok(())
            };
            session.active.store(false, Ordering::SeqCst);
            locked.stop();
            sessions.remove(&guild_id);
            drop(locked);
            player::refresh(&ctx.http, &self.catalog, &stopped, true).await;
            tokio::time::timeout(Duration::from_secs(5), manager.remove(guild_id))
                .await
                .map_err(|_| "Playback stopped, but leaving the voice channel timed out.")?
                .map_err(|_| "Playback stopped, but the bot could not leave the voice channel.")?;
            drop(sessions);
            player::refresh(&ctx.http, &self.catalog, &stopped, true).await;
            persistence.map_err(|_|"Playback stopped, but its stop intent could not be saved. Check storage before restarting Music.")?;
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
            "loop" => {
                session.looping.fetch_xor(true, Ordering::SeqCst);
            }
            "pause" => {
                handle.pause().map_err(|_| "Could not pause playback.")?;
                session.paused.store(true, Ordering::SeqCst);
            }
            "resume" => {
                if handle
                    .get_info()
                    .await
                    .map(|s| s.playing.is_done())
                    .unwrap_or(true)
                {
                    let next = self
                        .selection(session)
                        .await?
                        .ok_or("No recordings match the listeners' preferences.")?;
                    play_track(
                        &mut locked,
                        Advance {
                            call: Arc::downgrade(&call),
                            catalog: self.catalog.clone(),
                            session: session.clone(),
                            http: ctx.http.clone(),
                            text_channel: command.channel_id,
                            generation: 0,
                        },
                        next,
                    )?;
                } else {
                    handle.play().map_err(|_| "Could not resume playback.")?;
                }
                session.paused.store(false, Ordering::SeqCst);
            }
            _ => return Err("Unknown command.".into()),
        }
        let refreshed = session.clone();
        drop(locked);
        drop(sessions);
        player::refresh(&ctx.http, &self.catalog, &refreshed, false).await;
        Ok("Playback control applied.".into())
    }
}

#[async_trait]
impl EventHandler for Bot {
    async fn ready(&self, ctx: Context, _: Ready) {
        self.start_background(ctx).await;
        println!("Vozen Music connected to Discord.");
    }

    async fn voice_state_update(&self, ctx: Context, old: Option<VoiceState>, new: VoiceState) {
        if let Some(guild) = new.guild_id {
            if new.user_id == ctx.cache.current_user().id
                && old
                    .as_ref()
                    .and_then(|s| s.channel_id)
                    .is_some_and(|channel| Some(channel) != new.channel_id)
            {
                let session = self.sessions.lock().await.get(&guild).cloned();
                if let Some(session) = session
                    && let Err(error) = player::remove(&ctx.http, &session).await
                {
                    eprintln!("Could not delete the previous voice-channel player: {error}");
                }
            }
            let entered = new.channel_id.is_some()
                && old.as_ref().and_then(|s| s.channel_id) != new.channel_id
                && new
                    .member
                    .as_ref()
                    .map(|m| !m.user.bot)
                    .or_else(|| ctx.cache.user(new.user_id).map(|u| !u.bot))
                    .unwrap_or(false);
            if let Err(e) = self.reconcile(&ctx, guild, entered).await {
                eprintln!("Voice-state reconciliation failed: {e}");
            }
        }
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        let interaction = match interaction {
            Interaction::Component(button)
                if button.data.custom_id.starts_with("vozen:action:") =>
            {
                if player::acknowledge_button(&ctx.http, &button)
                    .await
                    .is_err()
                {
                    return;
                }
                let result: Result<String, Error> = async {
                    let parts: Vec<_> = button.data.custom_id.split(':').collect();
                    if parts.len() != 5 {
                        return Err("Invalid player control.".into());
                    }
                    let guild = button.guild_id.ok_or("Use this in a server.")?;
                    {
                        let sessions = self.sessions.lock().await;
                        let session = sessions.get(&guild).ok_or("Player stopped.")?;
                        if session.control_id.to_string() != parts[3]
                            || session.generation.load(Ordering::SeqCst).to_string() != parts[4]
                        {
                            return Err("This player is outdated. Use /panel.".into());
                        }
                    }
                    if !["pause", "resume", "like", "dislike", "stop", "loop", "volume-down", "volume-up"].contains(&parts[2]) {
                        return Err("Unknown player control.".into());
                    }
                    let mut data = serde_json::to_value(&button)?;
                    data["data"] =
                        serde_json::json!({"id":"1","name":parts[2],"type":1,"options":[{"name":"player-id","type":3,"value":parts[3]},{"name":"generation","type":4,"value":parts[4].parse::<i64>()?}]});
                    let command: CommandInteraction = serde_json::from_value(data)?;
                    self.command(&ctx, &command).await
                }
                .await;
                if let Err(error) = player::finish_button(&ctx.http, &button, result).await {
                    eprintln!("Could not respond to a player control: {error}");
                }
                return;
            }
            Interaction::Component(button)
                if button.data.custom_id.starts_with("vozen:skip:")
                    || button.data.custom_id.starts_with("vozen:previous:") =>
            {
                if player::acknowledge_button(&ctx.http, &button)
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
                    self.request_skip(
                        &ctx,
                        guild,
                        member,
                        button.channel_id,
                        button.id.get(),
                        Some(target),
                    )
                    .await
                }
                .await;
                if let Err(error) = player::finish_button(&ctx.http, &button, result).await {
                    eprintln!("Could not respond to a player control: {error}");
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
        let mut content = match result {
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
            if let Ok(votes) = player::vote_progress(session)
                && !votes.is_empty()
            {
                content.push_str(&format!("\n{votes}"));
            }
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
    if !args.is_empty()
        && args != ["--check"]
        && args != ["--check-data"]
        && args != ["--export-skip"]
    {
        return Err(
            "Usage: vozen-music [--check|--check-data|--export-skip], from the catalog directory."
                .into(),
        );
    }
    let mut loaded = Catalog::load(Path::new("."))?;
    if args.is_empty() {
        let tracks = loaded
            .stations
            .iter()
            .flat_map(|s| s.tracks.clone())
            .collect();
        loaded.stations.push(vozen_music::Station {
            id: "my".into(),
            name: "My Station".into(),
            tracks,
        });
    }
    let catalog = Arc::new(loaded);
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
    if args == ["--check-data"] || args == ["--export-skip"] {
        let store = SettingsStore::load(Path::new("data/skip-settings.json").to_path_buf())?;
        if args == ["--export-skip"] {
            println!(
                "{}",
                serde_json::to_string_pretty(&store.db.export_legacy()?)?
            );
        } else {
            println!(
                "Music SQLite data validated; {} configured server(s).",
                store.db.servers()?.len()
            );
        }
        return Ok(());
    }
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
        background: Arc::new(AtomicBool::new(false)),
        guild,
        sessions: Arc::new(Mutex::new(HashMap::new())),
        settings: Arc::new(Mutex::new(SettingsStore::load(
            Path::new("data/skip-settings.json").to_path_buf(),
        )?)),
        play_cooldowns: Arc::default(),
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
    let mut dashboard =
        dashboard::start(bot.clone(), client.cache.clone(), client.http.clone()).await?;
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
    let outcome: Result<(), Error> = tokio::select! {
        result = client.start() => { result.map_err(Into::into) },
        result = async {
            match &mut dashboard {
                Some(task) => task.await.map_err(|_| "Dashboard task stopped unexpectedly.")?,
                None => std::future::pending().await,
            }
        } => {
            client.shard_manager.shutdown_all().await;
            result.and(Err("Dashboard stopped unexpectedly.".into()))
        }
    };
    let sessions: Vec<_> = bot.sessions.lock().await.values().cloned().collect();
    for session in &sessions {
        session.active.store(false, Ordering::SeqCst);
    }
    if tokio::time::timeout(Duration::from_secs(10), async {
        for session in sessions {
            if let Err(error) = player::remove(&client.http, &session).await {
                eprintln!("Could not delete the player during shutdown: {error}");
            }
        }
    })
    .await
    .is_err()
    {
        eprintln!("Player cleanup during shutdown timed out.");
    }
    if let Some(task) = dashboard {
        task.abort();
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_commands_expose_english_names_descriptions_and_station_option() {
        let catalog = serde_json::from_str(include_str!("../catalog.json")).unwrap();
        let bot = Bot {
            catalog: Arc::new(catalog),
            background: Arc::new(AtomicBool::new(false)),
            guild: Some(GuildId::new(1)),
            sessions: Arc::new(Mutex::new(HashMap::new())),
            settings: Arc::new(Mutex::new(
                SettingsStore::load(std::env::temp_dir().join(format!(
                        "vozen-unused-{}.json",
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .as_nanos()
                    )))
                .unwrap(),
            )),
            play_cooldowns: Arc::default(),
        };
        let commands = serde_json::to_value(bot.commands()).unwrap();
        let commands = commands.as_array().unwrap();
        let names: Vec<_> = commands
            .iter()
            .map(|c| c["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            &names[..8],
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
                assert_eq!(option["required"], command["name"] != "play");
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
        handler.session.looping.store(true, Ordering::SeqCst);
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
    async fn natural_end_repeats_only_when_loop_is_enabled_and_stale_events_do_nothing() {
        let catalog = Arc::new(Catalog::load(Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap());
        for looping in [false, true] {
            let call = Arc::new(Mutex::new(Call::standalone(
                GuildId::new(1),
                UserId::new(2),
            )));
            let mut handler = advance(Arc::downgrade(&call), true);
            handler.catalog = catalog.clone();
            handler.session.looping.store(looping, Ordering::SeqCst);
            handler.act(&EventContext::Track(&[])).await;
            assert_eq!(
                handler.session.current.load(Ordering::SeqCst),
                usize::from(!looping)
            );
            assert_eq!(handler.session.generation.load(Ordering::SeqCst), 1);
            assert_eq!(
                handler.session.history.lock().unwrap().len(),
                usize::from(!looping)
            );
            handler.act(&EventContext::Track(&[])).await;
            assert_eq!(handler.session.generation.load(Ordering::SeqCst), 1);
            if looping {
                for generation in 1..=3 {
                    handler.generation = generation;
                    handler.act(&EventContext::Track(&[])).await;
                    assert_eq!(handler.session.current.load(Ordering::SeqCst), 0);
                    assert!(handler.session.looping.load(Ordering::SeqCst));
                    assert_eq!(
                        handler.session.generation.load(Ordering::SeqCst),
                        generation + 1
                    );
                }
                handler.session.looping.fetch_xor(true, Ordering::SeqCst);
                handler.generation = 4;
                handler.act(&EventContext::Track(&[])).await;
                assert_eq!(handler.session.current.load(Ordering::SeqCst), 1);
                assert!(!handler.session.looping.load(Ordering::SeqCst));
            }
            handler.session.active.store(false, Ordering::SeqCst);
            call.lock().await.stop();
        }
    }

    #[test]
    fn navigation_keeps_history_loop_and_timer_and_resets_both_vote_directions() {
        let session = Session::new(0, 0, 1, SkipSettings::default(), None);
        session.set_track(1).unwrap();
        session.set_track(2).unwrap();
        session.set_track(2).unwrap();
        session.looping.store(true, Ordering::SeqCst);
        session.volume.store(40, Ordering::SeqCst);
        session.sleep_until.store(1234, Ordering::SeqCst);
        session.votes_for(false).lock().unwrap().voters.insert(10);
        assert!(session.votes_for(true).lock().unwrap().voters.is_empty());
        session.votes_for(true).lock().unwrap().voters.insert(20);
        let previous = session
            .for_track_change(1, 2, SkipSettings::default(), true)
            .unwrap();
        previous.set_track(1).unwrap();
        assert_eq!(previous.current.load(Ordering::SeqCst), 1);
        assert_eq!(*previous.history.lock().unwrap(), VecDeque::from([0]));
        assert!(previous.looping.load(Ordering::SeqCst));
        assert_eq!(previous.volume.load(Ordering::SeqCst), 40);
        assert_eq!(previous.sleep_until.load(Ordering::SeqCst), 1234);
        assert_eq!(previous.control_id, 2);
        assert!(
            session
                .for_track_change(0, 2, SkipSettings::default(), true)
                .is_err()
        );
        let next = previous
            .for_track_change(3, 3, SkipSettings::default(), false)
            .unwrap();
        next.set_track(3).unwrap();
        assert_eq!(*next.history.lock().unwrap(), VecDeque::from([0, 1]));
        session.reset_votes(7).unwrap();
        for direction in [false, true] {
            assert!(
                session
                    .votes_for(direction)
                    .lock()
                    .unwrap()
                    .voters
                    .is_empty()
            );
        }
        for index in 3..=105 {
            session.set_track(index).unwrap();
        }
        let history = session.history.lock().unwrap();
        assert_eq!(history.len(), 100);
        assert_eq!(history.back(), Some(&104));
        assert_eq!(history.front(), Some(&5));
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
