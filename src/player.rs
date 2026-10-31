use super::{Catalog, Error, Session};
use serenity::all::*;
use std::sync::{Arc, atomic::Ordering};
use vozen_music::skip::SkipMode;

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
        .disabled(stopped)
    };
    Ok(vec![CreateActionRow::Buttons(vec![
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
            .style(ButtonStyle::Primary)
            .disabled(stopped),
        action("like", "Like"),
        action("dislike", "Dislike"),
        action("stop", "Stop"),
    ])])
}

fn content(catalog: &Catalog, session: &Session, stopped: bool) -> Result<String, Error> {
    let station = &catalog.stations[session.station];
    if stopped {
        return Ok(format!(
            "### {}\nPlayback stopped. Use /play to start a station.",
            station.name
        ));
    }
    let track = &station.tracks[session.current.load(Ordering::SeqCst)];
    let policy = session
        .policy
        .lock()
        .map_err(|_| "Player settings are unavailable.")?;
    let tip = if session.tips.load(Ordering::SeqCst) {
        "\nTip: /like and /dislike shape future tracks; /library shows your ratings."
    } else {
        ""
    };
    Ok(format!(
        "### {}\n**{}** — {}\n{}\nSource: <{}> • {}\nJoin the same voice channel • One vote per person • Votes reset every track{}",
        station.name,
        track.title,
        track.artist,
        policy.summary(),
        track.source_url,
        track.license,
        tip
    ))
}

pub fn message(catalog: &Catalog, session: &Session) -> Result<CreateMessage, Error> {
    Ok(CreateMessage::new()
        .content(content(catalog, session, false)?)
        .components(controls(
            session,
            session.generation.load(Ordering::SeqCst).max(1),
            false,
        )?)
        .allowed_mentions(CreateAllowedMentions::new()))
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
            .content(content(catalog, session, stopped)?)
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
        policy.mode = SkipMode::Majority;
        *session.policy.lock().unwrap() = policy;
        let buttons = serde_json::to_value(controls(&session, 3, false).unwrap()).unwrap();
        assert_eq!(buttons[0]["components"][1]["label"], "Vote to skip");
        let disabled = serde_json::to_value(controls(&session, 3, true).unwrap()).unwrap();
        assert_eq!(disabled[0]["components"][1]["disabled"], true);
    }
}
