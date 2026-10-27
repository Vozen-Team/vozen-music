use super::*;
use std::collections::BTreeSet;
use vozen_music::{
    next_track,
    skip::{MajorityScope, SkipAccess, SkipMode},
};

pub fn definition() -> CreateCommand {
    CreateCommand::new("skip-settings")
        .description("View or change this server's skip settings")
        .default_member_permissions(Permissions::MANAGE_GUILD)
        .add_option(
            CreateCommandOption::new(CommandOptionType::String, "access", "Who may skip or vote")
                .add_string_choice("Everyone in the call", "everyone")
                .add_string_choice("Moderators only", "moderators"),
        )
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::String,
                "mode",
                "Skip immediately or require a majority",
            )
            .add_string_choice("Instant", "instant")
            .add_string_choice("Majority vote", "majority"),
        )
        .add_option(
            CreateCommandOption::new(
                CommandOptionType::String,
                "majority-scope",
                "Who counts towards the majority",
            )
            .add_string_choice("All humans in the call", "call")
            .add_string_choice("Eligible voters only", "eligible"),
        )
        .add_option(CreateCommandOption::new(
            CommandOptionType::Role,
            "add-role",
            "Add an eligible moderator role",
        ))
        .add_option(CreateCommandOption::new(
            CommandOptionType::Role,
            "remove-role",
            "Remove an eligible moderator role",
        ))
        .add_option(CreateCommandOption::new(
            CommandOptionType::Boolean,
            "clear-roles",
            "Clear moderator roles and use server moderation permissions",
        ))
}

impl Bot {
    fn member_flags(
        &self,
        ctx: &Context,
        guild: GuildId,
        member: &Member,
    ) -> Result<(bool, bool), Error> {
        let guild = ctx
            .cache
            .guild(guild)
            .ok_or("The server is still loading. Please try again.")?;
        let mut permissions = guild
            .roles
            .get(&RoleId::new(guild.id.get()))
            .map(|r| r.permissions)
            .unwrap_or_default();
        for id in &member.roles {
            permissions |= guild
                .roles
                .get(id)
                .map(|r| r.permissions)
                .unwrap_or_default();
        }
        let manager = guild.owner_id == member.user.id
            || permissions.intersects(Permissions::ADMINISTRATOR | Permissions::MANAGE_GUILD);
        let moderator = permissions.intersects(
            Permissions::MODERATE_MEMBERS
                | Permissions::KICK_MEMBERS
                | Permissions::BAN_MEMBERS
                | Permissions::MANAGE_MESSAGES,
        );
        Ok((manager, moderator))
    }

    pub async fn configure_skip(
        &self,
        ctx: &Context,
        command: &CommandInteraction,
    ) -> Result<String, Error> {
        let guild = command
            .guild_id
            .ok_or("This command can only be used in a server.")?;
        let member = command
            .member
            .as_deref()
            .ok_or("Server member information is unavailable.")?;
        if !self.member_flags(ctx, guild, member)?.0 {
            return Err("You need Manage Server permission to change skip settings.".into());
        }
        let option = |name: &str| {
            command
                .data
                .options
                .iter()
                .find(|o| o.name == name)
                .map(|o| &o.value)
        };
        let add = option("add-role").and_then(|v| v.as_role_id());
        let remove = option("remove-role").and_then(|v| v.as_role_id());
        if add.is_some() && add == remove {
            return Err("Choose different roles to add and remove.".into());
        }
        if let Some(role) = add {
            let server = ctx
                .cache
                .guild(guild)
                .ok_or("The server is still loading.")?;
            if role.get() == guild.get() || !server.roles.contains_key(&role) {
                return Err("Choose an existing moderator role other than @everyone.".into());
            }
        }
        let sessions = self.sessions.lock().await;
        let policy = {
            let mut store = self.settings.lock().await;
            let mut policy = store.get(guild.get());
            if command.data.options.is_empty() {
                return Ok(policy.summary());
            }
            if let Some(value) = option("access").and_then(|v| v.as_str()) {
                policy.access = match value {
                    "everyone" => SkipAccess::Everyone,
                    "moderators" => SkipAccess::Moderators,
                    _ => return Err("Invalid skip access.".into()),
                };
            }
            if let Some(value) = option("mode").and_then(|v| v.as_str()) {
                policy.mode = match value {
                    "instant" => SkipMode::Instant,
                    "majority" => SkipMode::Majority,
                    _ => return Err("Invalid skip mode.".into()),
                };
            }
            if let Some(value) = option("majority-scope").and_then(|v| v.as_str()) {
                policy.majority_scope = match value {
                    "call" => MajorityScope::Call,
                    "eligible" => MajorityScope::Eligible,
                    _ => return Err("Invalid majority scope.".into()),
                };
            }
            if option("clear-roles").and_then(|v| v.as_bool()) == Some(true) {
                policy.moderator_roles.clear();
            }
            if let Some(id) = add {
                policy.moderator_roles.insert(id.get());
            }
            if let Some(id) = remove {
                policy.moderator_roles.remove(&id.get());
            }
            store.set(guild.get(), policy.clone()).map_err(|error| {
                eprintln!("Could not save skip settings: {error}");
                "Could not save skip settings. Check the bot's data storage."
            })?;
            policy
        };
        let session = {
            if let Some(session) = sessions.get(&guild) {
                *session
                    .policy
                    .lock()
                    .map_err(|_| "Player settings are unavailable.")? = policy.clone();
                session
                    .votes
                    .lock()
                    .map_err(|_| "Skip votes are unavailable.")?
                    .reset(session.generation.load(Ordering::SeqCst));
            }
            sessions.get(&guild).cloned()
        };
        drop(sessions);
        if let Some(session) = session {
            player::refresh(&ctx.http, &self.catalog, &session, false).await;
        }
        Ok(format!(
            "Skip settings saved. Current votes were cleared.\n{}",
            policy.summary()
        ))
    }

    fn voice_ids(
        &self,
        ctx: &Context,
        guild: GuildId,
        channel: ChannelId,
    ) -> Result<BTreeSet<u64>, Error> {
        let guild = ctx
            .cache
            .guild(guild)
            .ok_or("The server is still loading. Please try again.")?;
        Ok(guild
            .voice_states
            .iter()
            .filter(|(_, state)| state.channel_id == Some(channel))
            .map(|(id, _)| id.get())
            .collect())
    }

    async fn listeners(
        &self,
        ctx: &Context,
        guild: GuildId,
        channel: ChannelId,
        actor: &Member,
        restricted: bool,
    ) -> Result<Vec<Member>, Error> {
        let snapshot = {
            let server = ctx
                .cache
                .guild(guild)
                .ok_or("The server is still loading.")?;
            server
                .voice_states
                .iter()
                .filter(|(_, s)| s.channel_id == Some(channel))
                .map(|(id, s)| {
                    (
                        *id,
                        s.member.clone().or_else(|| server.members.get(id).cloned()),
                    )
                })
                .collect::<Vec<_>>()
        };
        let mut members = Vec::with_capacity(snapshot.len());
        for (id, cached) in snapshot {
            let member = if id == actor.user.id {
                actor.clone()
            } else if cached.as_ref().is_some_and(|m| m.user.bot || !restricted) {
                cached.unwrap()
            } else {
                guild
                    .member(&ctx.http, id)
                    .await
                    .map_err(|_| "Could not verify the current listeners. Please try again.")?
            };
            members.push(member);
        }
        Ok(members)
    }

    pub async fn request_skip(
        &self,
        ctx: &Context,
        guild: GuildId,
        member: &Member,
        interaction_id: u64,
        target: Option<(u64, usize)>,
    ) -> Result<String, Error> {
        if self.guild.is_some_and(|allowed| guild != allowed) {
            return Err("This version is limited to the development server.".into());
        }
        let policy = self.settings.lock().await.get(guild.get());
        let roles = member.roles.iter().map(|id| id.get()).collect::<Vec<_>>();
        let (manager_permission, moderator) = self.member_flags(ctx, guild, member)?;
        if member.user.bot || !policy.allows(manager_permission, moderator, &roles) {
            return Err("Only eligible moderators in the voice channel may skip or vote.".into());
        }
        let manager = songbird::get(ctx)
            .await
            .ok_or("The voice connection service is unavailable.")?;
        let call = manager
            .get(guild)
            .ok_or("No station is active. Use /play first.")?;
        let channel = call
            .lock()
            .await
            .current_channel()
            .map(|id| ChannelId::new(id.0.get()))
            .ok_or("The bot is not in a voice channel.")?;
        if !self
            .voice_ids(ctx, guild, channel)?
            .contains(&member.user.id.get())
        {
            return Err("Join the same voice channel as Vozen Music to skip or vote.".into());
        }
        let expected_target = {
            let sessions = self.sessions.lock().await;
            let session = sessions
                .get(&guild)
                .ok_or("No station is active. Use /play first.")?;
            if !session.active.load(Ordering::SeqCst)
                || target.is_some_and(|t| {
                    t != (
                        session.control_id,
                        session.generation.load(Ordering::SeqCst),
                    )
                })
            {
                return Err(
                    "This player is outdated. Use the latest player or /nowplaying.".into(),
                );
            }
            if policy.mode == SkipMode::Majority {
                session
                    .votes
                    .lock()
                    .map_err(|_| "Skip votes are unavailable.")?
                    .begin_request(
                        member.user.id.get(),
                        std::time::Instant::now(),
                        &self.voice_ids(ctx, guild, channel)?,
                    )?;
            }
            (
                session.control_id,
                session.generation.load(Ordering::SeqCst),
            )
        };
        // Resolve membership outside playback locks; a slow Discord API must not block track endings.
        let listeners = if policy.mode == SkipMode::Majority {
            tokio::time::timeout(
                Duration::from_secs(15),
                self.listeners(
                    ctx,
                    guild,
                    channel,
                    member,
                    policy.access == SkipAccess::Moderators,
                ),
            )
            .await
            .map_err(|_| "Listener verification timed out. Please try again.")??
        } else {
            vec![member.clone()]
        };
        let humans = listeners.iter().filter(|m| !m.user.bot).count();
        let mut eligible = BTreeSet::new();
        for listener in &listeners {
            let (manager, moderator) = self.member_flags(ctx, guild, listener)?;
            let roles = listener.roles.iter().map(|id| id.get()).collect::<Vec<_>>();
            if !listener.user.bot && policy.allows(manager, moderator, &roles) {
                eligible.insert(listener.user.id.get());
            }
        }
        let mut sessions = self.sessions.lock().await;
        if self.settings.lock().await.get(guild.get()) != policy {
            return Err("Skip settings changed. Please try again.".into());
        }
        let session = sessions
            .get(&guild)
            .cloned()
            .ok_or("No station is active. Use /play first.")?;
        let text_channel = session
            .panel
            .as_ref()
            .map(|panel| panel.channel)
            .unwrap_or(channel);
        let mut locked = call.lock().await;
        let voice_ids = self.voice_ids(ctx, guild, channel)?;
        if locked.current_channel().map(|id| id.0.get()) != Some(channel.get())
            || !voice_ids.contains(&member.user.id.get())
        {
            return Err("Join the same voice channel as Vozen Music to skip or vote.".into());
        }
        if policy.mode == SkipMode::Majority
            && voice_ids != listeners.iter().map(|m| m.user.id.get()).collect()
        {
            return Err("Voice membership changed. Please press Skip again.".into());
        }
        let generation = session.generation.load(Ordering::SeqCst);
        if !session.active.load(Ordering::SeqCst)
            || expected_target != (session.control_id, generation)
        {
            return Err("This player is outdated. Use the latest player or /nowplaying.".into());
        }
        let required = policy.required_votes(humans, eligible.len());
        let progress = session
            .votes
            .lock()
            .map_err(|_| "Skip votes are unavailable.")?
            .cast(generation, member.user.id.get(), &eligible, required)?;
        if !progress.passed() {
            drop(locked);
            drop(sessions);
            player::refresh(&ctx.http, &self.catalog, &session, false).await;
            let mut reply = format!(
                "{} Votes: {}/{}.",
                if progress.added {
                    "Vote recorded."
                } else {
                    "You already voted for this track."
                },
                progress.votes,
                progress.required
            );
            if eligible.len() < required {
                reply.push_str(&format!(" Only {} listeners are currently eligible; ask a server manager to review the majority scope.", eligible.len()));
            }
            return Ok(reply);
        }
        let next = next_track(
            session.current.load(Ordering::SeqCst),
            self.catalog.stations[session.station].tracks.len(),
        )
        .ok_or("The station is empty.")?;
        session.active.store(false, Ordering::SeqCst);
        let replacement = Arc::new(Session::new(
            session.station,
            next,
            interaction_id,
            policy,
            session.panel.clone(),
        ));
        let advance = Advance {
            call: Arc::downgrade(&call),
            catalog: self.catalog.clone(),
            session: replacement.clone(),
            http: ctx.http.clone(),
            text_channel,
            generation: 0,
        };
        play_track(&mut locked, advance, next)?;
        sessions.insert(guild, replacement.clone());
        drop(locked);
        drop(sessions);
        player::refresh(&ctx.http, &self.catalog, &replacement, false).await;
        Ok("Track skipped. Votes were reset for the next track.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn server_configuration_is_manager_only_and_exposes_both_scopes() {
        let command = serde_json::to_value(definition()).unwrap();
        assert_eq!(command["default_member_permissions"], "32");
        let options = command["options"].as_array().unwrap();
        assert_eq!(options.len(), 6);
        let scope = options
            .iter()
            .find(|o| o["name"] == "majority-scope")
            .unwrap();
        assert_eq!(scope["choices"][0]["value"], "call");
        assert_eq!(scope["choices"][1]["value"], "eligible");
    }
}
