use crate::{Error, skip::SkipSettings};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RadioMode {
    #[default]
    Session,
    Always,
    AutoJoin,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ChannelRule {
    pub allow: bool,
    #[serde(with = "id_set")]
    pub channels: BTreeSet<u64>,
}
impl ChannelRule {
    pub fn permits(&self, channel: u64) -> bool {
        self.channels.is_empty() && !self.allow || self.channels.contains(&channel) == self.allow
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct ServerSettings {
    pub revision: u64,
    pub skip: SkipSettings,
    pub volume: u8,
    pub default_station: String,
    pub radio_mode: RadioMode,
    #[serde(with = "optional_id")]
    pub voice_channel: Option<u64>,
    pub voice_rules: ChannelRule,
    pub text_rules: ChannelRule,
    #[serde(with = "id_set")]
    pub dj_roles: BTreeSet<u64>,
    pub djs_only: bool,
    pub player_locked: bool,
    pub announcements: bool,
    #[serde(with = "optional_id")]
    pub announcement_channel: Option<u64>,
    pub tips: bool,
    pub statistics: bool,
}
impl Default for ServerSettings {
    fn default() -> Self {
        Self {
            revision: 0,
            skip: SkipSettings::default(),
            volume: 100,
            default_station: "lofi".into(),
            radio_mode: RadioMode::Session,
            voice_channel: None,
            voice_rules: ChannelRule::default(),
            text_rules: ChannelRule::default(),
            dj_roles: BTreeSet::new(),
            djs_only: false,
            player_locked: false,
            announcements: true,
            announcement_channel: None,
            tips: false,
            statistics: false,
        }
    }
}
impl ServerSettings {
    pub fn permits_message_channel(&self, channel: u64, voice: bool) -> bool {
        if voice {
            self.voice_rules.permits(channel)
                && self.voice_channel.is_none_or(|fixed| fixed == channel)
        } else {
            self.text_rules.permits(channel)
        }
    }

    pub fn validate_resources(
        &self,
        guild: u64,
        roles: &BTreeSet<u64>,
        voice: &BTreeSet<u64>,
        text: &BTreeSet<u64>,
        stations: &BTreeSet<String>,
    ) -> Result<(), Error> {
        self.validate(guild)?;
        if !stations.contains(&self.default_station)
            || self
                .dj_roles
                .iter()
                .chain(&self.skip.moderator_roles)
                .any(|r| !roles.contains(r))
            || self.voice_rules.channels.iter().any(|c| !voice.contains(c))
            || self.text_rules.channels.iter().any(|c| !text.contains(c))
            || self.voice_channel.is_some_and(|c| !voice.contains(&c))
            || self
                .announcement_channel
                .is_some_and(|c| !text.contains(&c))
        {
            return Err("A selected role, channel or station is unavailable.".into());
        }
        if self.radio_mode == RadioMode::AutoJoin && self.announcement_channel.is_none() {
            return Err("Choose a text channel for Auto-join controls and errors.".into());
        }
        Ok(())
    }
    pub fn validate(&self, guild: u64) -> Result<(), Error> {
        if guild == 0
            || self.volume > 100
            || self.default_station.is_empty()
            || self.dj_roles.len() > 50
            || self.skip.moderator_roles.len() > 50
            || self.voice_rules.channels.len() > 100
            || self.text_rules.channels.len() > 100
            || self.dj_roles.contains(&0)
            || self.dj_roles.contains(&guild)
            || self.skip.moderator_roles.contains(&0)
            || self.skip.moderator_roles.contains(&guild)
            || self.voice_rules.channels.contains(&0)
            || self.text_rules.channels.contains(&0)
            || self.voice_channel == Some(0)
            || self.announcement_channel == Some(0)
        {
            return Err("Invalid server settings.".into());
        }
        if self.radio_mode == RadioMode::AutoJoin && self.voice_channel.is_none() {
            return Err("Choose a voice channel for Auto-join.".into());
        }
        if self
            .voice_channel
            .is_some_and(|id| !self.voice_rules.permits(id))
        {
            return Err("The selected voice channel is blocked by your rules.".into());
        }
        if self
            .announcement_channel
            .is_some_and(|id| !self.text_rules.permits(id))
        {
            return Err("The announcement channel is blocked by your rules.".into());
        }
        Ok(())
    }
    pub fn permits_control(&self, manager: bool, roles: &[u64]) -> bool {
        manager
            || !(self.player_locked || self.djs_only)
            || roles.iter().any(|r| self.dj_roles.contains(r))
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Privacy {
    pub enabled: bool,
    pub visible: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlaybackIntent {
    pub guild: u64,
    pub channel: u64,
    pub text_channel: u64,
    pub station: String,
    pub track: String,
    pub active: bool,
    pub sleep_until: Option<i64>,
}

pub struct Store {
    db: Connection,
}
impl Store {
    pub fn export_legacy(&self) -> Result<serde_json::Value, Error> {
        let mut value = serde_json::to_value(self.export_skip()?)?;
        for policy in value
            .as_object_mut()
            .ok_or("Invalid settings export.")?
            .values_mut()
        {
            let roles = policy["moderator_roles"]
                .as_array_mut()
                .ok_or("Invalid role export.")?;
            for role in roles {
                let id = role.as_str().ok_or("Invalid role ID.")?.parse::<u64>()?;
                *role = serde_json::json!(id);
            }
        }
        Ok(value)
    }
    pub fn export_skip(&self) -> Result<BTreeMap<u64, SkipSettings>, Error> {
        self.servers()?
            .into_iter()
            .map(|g| Ok((g, self.settings(g)?.skip)))
            .collect()
    }
    pub fn open(path: &Path, legacy: &Path) -> Result<Self, Error> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let mut db = Connection::open(path)?;
        db.busy_timeout(std::time::Duration::from_secs(3))?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;
          CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT NOT NULL);
          CREATE TABLE IF NOT EXISTS settings(guild TEXT PRIMARY KEY, revision INTEGER NOT NULL, body TEXT NOT NULL);
          CREATE TABLE IF NOT EXISTS ratings(guild TEXT,user TEXT,track TEXT,value INTEGER NOT NULL CHECK(value IN (-1,1)),PRIMARY KEY(guild,user,track));
          CREATE TABLE IF NOT EXISTS privacy(guild TEXT,user TEXT,enabled INTEGER NOT NULL,visible INTEGER NOT NULL,seconds INTEGER NOT NULL DEFAULT 0,PRIMARY KEY(guild,user));
          CREATE TABLE IF NOT EXISTS track_listening(guild TEXT,track TEXT,seconds INTEGER NOT NULL,PRIMARY KEY(guild,track));
          CREATE TABLE IF NOT EXISTS playback(guild TEXT PRIMARY KEY,body TEXT NOT NULL);")?;
        let imported: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM metadata WHERE key='legacy_imported')",
            [],
            |r| r.get(0),
        )?;
        if !imported {
            let policies: BTreeMap<u64, SkipSettings> = match std::fs::read(legacy) {
                Ok(bytes) => serde_json::from_slice(&bytes)?,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
                Err(e) => return Err(e.into()),
            };
            let tx = db.transaction()?;
            for (guild, skip) in policies {
                let s = ServerSettings {
                    skip,
                    ..Default::default()
                };
                s.validate(guild)?;
                tx.execute(
                    "INSERT OR IGNORE INTO settings VALUES(?1,0,?2)",
                    params![guild.to_string(), serde_json::to_string(&s)?],
                )?;
            }
            tx.execute("INSERT INTO metadata VALUES('legacy_imported','1')", [])?;
            tx.commit()?;
        }
        let integrity: String = db.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
        if integrity != "ok" {
            return Err("Music database integrity check failed.".into());
        }
        Ok(Self { db })
    }
    pub fn settings(&self, guild: u64) -> Result<ServerSettings, Error> {
        let body: Option<String> = self
            .db
            .query_row(
                "SELECT body FROM settings WHERE guild=?1",
                [guild.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        let s: ServerSettings = body
            .map(|b| serde_json::from_str(&b))
            .transpose()?
            .unwrap_or_default();
        s.validate(guild)?;
        Ok(s)
    }
    pub fn save(
        &mut self,
        guild: u64,
        mut settings: ServerSettings,
    ) -> Result<ServerSettings, Error> {
        settings.validate(guild)?;
        let tx = self.db.transaction()?;
        let actual: u64 = tx
            .query_row(
                "SELECT revision FROM settings WHERE guild=?1",
                [guild.to_string()],
                |r| r.get::<_, i64>(0),
            )
            .optional()?
            .unwrap_or(0) as u64;
        if actual != settings.revision {
            return Err("Settings changed. Reload before saving.".into());
        }
        settings.revision += 1;
        tx.execute("INSERT INTO settings VALUES(?1,?2,?3) ON CONFLICT(guild) DO UPDATE SET revision=excluded.revision,body=excluded.body",params![guild.to_string(),settings.revision as i64,serde_json::to_string(&settings)?])?;
        tx.commit()?;
        Ok(settings)
    }
    pub fn rate(&self, g: u64, u: u64, track: &str, value: i8) -> Result<(), Error> {
        if g == 0 || u == 0 || track.is_empty() || !(-1..=1).contains(&value) {
            return Err("Invalid rating.".into());
        }
        if value == 0 {
            self.db.execute(
                "DELETE FROM ratings WHERE guild=?1 AND user=?2 AND track=?3",
                params![g.to_string(), u.to_string(), track],
            )?;
        } else {
            self.db.execute("INSERT INTO ratings VALUES(?1,?2,?3,?4) ON CONFLICT(guild,user,track) DO UPDATE SET value=excluded.value",params![g.to_string(),u.to_string(),track,value])?;
        }
        Ok(())
    }
    pub fn ratings(
        &self,
        g: u64,
        users: &BTreeSet<u64>,
    ) -> Result<BTreeMap<u64, BTreeMap<String, i8>>, Error> {
        let mut statement = self
            .db
            .prepare("SELECT user,track,value FROM ratings WHERE guild=?1")?;
        let mut result = BTreeMap::new();
        for row in statement.query_map([g.to_string()], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i8>(2)?,
            ))
        })? {
            let (u, t, v) = row?;
            let u = u.parse()?;
            if users.contains(&u) {
                result.entry(u).or_insert_with(BTreeMap::new).insert(t, v);
            }
        }
        Ok(result)
    }
    pub fn toggle_like(&self, g: u64, u: u64, track: &str) -> Result<bool, Error> {
        let liked: bool = self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM ratings WHERE guild=?1 AND user=?2 AND track=?3 AND value=1)",
            params![g.to_string(), u.to_string(), track],
            |row| row.get(0),
        )?;
        self.rate(g, u, track, if liked { 0 } else { 1 })?;
        Ok(!liked)
    }
    pub fn privacy(&self, g: u64, u: u64) -> Result<(Privacy, u64), Error> {
        Ok(self
            .db
            .query_row(
                "SELECT enabled,visible,seconds FROM privacy WHERE guild=?1 AND user=?2",
                params![g.to_string(), u.to_string()],
                |r| {
                    Ok((
                        Privacy {
                            enabled: r.get(0)?,
                            visible: r.get(1)?,
                        },
                        r.get::<_, i64>(2)? as u64,
                    ))
                },
            )
            .optional()?
            .unwrap_or_default())
    }
    pub fn set_privacy(&self, g: u64, u: u64, p: Privacy, erase: bool) -> Result<(), Error> {
        self.db.execute("INSERT INTO privacy(guild,user,enabled,visible) VALUES(?1,?2,?3,?4) ON CONFLICT(guild,user) DO UPDATE SET enabled=excluded.enabled,visible=excluded.visible,seconds=CASE WHEN ?5 THEN 0 ELSE privacy.seconds END",params![g.to_string(),u.to_string(),p.enabled,p.visible,erase])?;
        Ok(())
    }
    pub fn record_listening(
        &mut self,
        guild: u64,
        track: &str,
        listeners: &BTreeSet<u64>,
        seconds: u64,
    ) -> Result<(), Error> {
        if listeners.is_empty() || seconds == 0 {
            return Ok(());
        }
        let guild = guild.to_string();
        let seconds = seconds.min(60) as i64;
        let tx = self.db.transaction()?;
        let mut total = 0;
        {
            let mut add_personal = tx.prepare_cached(
                "UPDATE privacy SET seconds=seconds+?3 WHERE guild=?1 AND user=?2 AND enabled=1",
            )?;
            for user in listeners {
                total += add_personal.execute(params![guild, user.to_string(), seconds])? as i64
                    * seconds;
            }
        }
        if total > 0 {
            tx.execute(
                "INSERT INTO track_listening(guild,track,seconds) VALUES(?1,?2,?3)
                 ON CONFLICT(guild,track) DO UPDATE SET seconds=seconds+excluded.seconds",
                params![guild, track, total],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn top_track(&self, guild: u64) -> Result<Option<(String, i64, i64)>, Error> {
        Ok(self
            .db
            .query_row(
                "SELECT track,seconds,SUM(seconds) OVER () FROM track_listening
             WHERE guild=?1 ORDER BY seconds DESC,track LIMIT 1",
                [guild.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?)
    }
    pub fn leaderboard(&self, g: u64) -> Result<Vec<(String, u64)>, Error> {
        let mut s=self.db.prepare("SELECT user,seconds FROM privacy WHERE guild=?1 AND enabled=1 AND visible=1 ORDER BY seconds DESC,user LIMIT 20")?;
        Ok(s.query_map([g.to_string()], |r| {
            Ok((r.get(0)?, r.get::<_, i64>(1)? as u64))
        })?
        .collect::<Result<_, _>>()?)
    }
    pub fn clear_ratings(&self, g: u64, u: u64) -> Result<(), Error> {
        self.db.execute(
            "DELETE FROM ratings WHERE guild=?1 AND user=?2",
            params![g.to_string(), u.to_string()],
        )?;
        Ok(())
    }
    pub fn intent(&self, intent: &PlaybackIntent) -> Result<(), Error> {
        self.db.execute("INSERT INTO playback VALUES(?1,?2) ON CONFLICT(guild) DO UPDATE SET body=excluded.body",params![intent.guild.to_string(),serde_json::to_string(intent)?])?;
        Ok(())
    }
    pub fn update_active_timer(&self, guild: u64, deadline: Option<i64>) -> Result<bool, Error> {
        Ok(self.db.execute(
            "UPDATE playback SET body=json_set(body, '$.sleep_until', ?2)
             WHERE guild=?1 AND json_extract(body, '$.active')=1",
            params![guild.to_string(), deadline],
        )? != 0)
    }
    pub fn intents(&self) -> Result<Vec<PlaybackIntent>, Error> {
        let mut s = self.db.prepare("SELECT body FROM playback")?;
        let rows = s
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        rows.iter().map(|s| Ok(serde_json::from_str(s)?)).collect()
    }
    pub fn servers(&self) -> Result<Vec<u64>, Error> {
        let mut s = self.db.prepare("SELECT guild FROM settings")?;
        let rows = s
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        rows.iter().map(|s| Ok(s.parse()?)).collect()
    }
}

pub fn candidates(
    tracks: &[String],
    ratings: &BTreeMap<u64, BTreeMap<String, i8>>,
    current: Option<&str>,
    personal: bool,
    turn: usize,
) -> Vec<(String, f64)> {
    let eligible: Vec<_> = tracks
        .iter()
        .filter(|t| !ratings.values().any(|r| r.get(*t) == Some(&-1)))
        .collect();
    let mut weights = BTreeMap::<String, f64>::new();
    for user in ratings.values() {
        let liked: Vec<_> = eligible
            .iter()
            .filter(|t| user.get(t.as_str()) == Some(&1))
            .collect();
        for t in &liked {
            *weights.entry((**t).clone()).or_default() += 1.0 / liked.len() as f64;
        }
    }
    let favorites = personal && turn % 5 != 4 && !weights.is_empty();
    let discovery =
        personal && turn % 5 == 4 && eligible.iter().any(|t| !weights.contains_key(t.as_str()));
    let pool: Vec<_> = eligible
        .into_iter()
        .filter(|t| {
            (!favorites || weights.contains_key(t.as_str()))
                && (!discovery || !weights.contains_key(t.as_str()))
        })
        .collect();
    let alternatives = pool.iter().any(|t| Some(t.as_str()) != current);
    pool.into_iter()
        .filter(|t| !alternatives || Some(t.as_str()) != current)
        .map(|t| {
            (
                t.clone(),
                if favorites {
                    weights.get(t.as_str()).copied().unwrap_or(0.0)
                } else {
                    1.0 + weights.get(t.as_str()).copied().unwrap_or(0.0)
                },
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn voice_chat_controls_use_voice_rules_without_bypassing_channel_limits() {
        let mut settings = ServerSettings::default();
        settings.text_rules.allow = true;
        settings.text_rules.channels.insert(10);
        assert!(settings.permits_message_channel(10, false));
        assert!(!settings.permits_message_channel(20, false));
        assert!(settings.permits_message_channel(20, true));
        settings.voice_rules.channels.insert(20);
        assert!(!settings.permits_message_channel(20, true));
        settings.voice_rules.allow = true;
        assert!(settings.permits_message_channel(20, true));
        assert!(!settings.permits_message_channel(30, true));
        settings.voice_channel = Some(30);
        assert!(!settings.permits_message_channel(20, true));
        assert!(settings.permits_message_channel(10, false));
    }

    fn store() -> Store {
        Store::open(Path::new(":memory:"), Path::new("missing-legacy-test.json")).unwrap()
    }
    #[test]
    fn revisions_ratings_privacy() {
        let mut s = store();
        let original = s.settings(1).unwrap();
        let mut next = original.clone();
        next.volume = 42;
        assert_eq!(s.save(1, next).unwrap().revision, 1);
        assert!(s.save(1, original).is_err());
        s.rate(1, 2, "a", 1).unwrap();
        s.rate(1, 2, "a", -1).unwrap();
        assert_eq!(s.ratings(1, &BTreeSet::from([2])).unwrap()[&2]["a"], -1);
        assert!(s.ratings(9, &BTreeSet::from([2])).unwrap().is_empty());
        s.record_listening(1, "a", &BTreeSet::from([2]), 30)
            .unwrap();
        assert_eq!(s.privacy(1, 2).unwrap().1, 0);
        assert!(s.top_track(1).unwrap().is_none());
        s.set_privacy(
            1,
            2,
            Privacy {
                enabled: true,
                visible: false,
            },
            false,
        )
        .unwrap();
        s.record_listening(1, "a", &BTreeSet::from([2, 3]), 30)
            .unwrap();
        s.record_listening(1, "b", &BTreeSet::from([2]), 10)
            .unwrap();
        assert_eq!(s.top_track(1).unwrap(), Some(("a".into(), 30, 40)));
        assert!(s.top_track(9).unwrap().is_none());
        assert!(s.leaderboard(1).unwrap().is_empty());
        s.set_privacy(
            1,
            2,
            Privacy {
                enabled: true,
                visible: true,
            },
            false,
        )
        .unwrap();
        assert_eq!(s.leaderboard(1).unwrap()[0].1, 40);
        s.set_privacy(1, 2, Privacy::default(), true).unwrap();
        assert_eq!(s.privacy(1, 2).unwrap().1, 0);
        assert_eq!(s.ratings(1, &BTreeSet::from([2])).unwrap()[&2]["a"], -1);
    }
    #[test]
    fn selection_and_rules() {
        let ids = vec!["a".into(), "b".into(), "c".into()];
        let r = BTreeMap::from([
            (1, BTreeMap::from([("a".into(), 1), ("b".into(), 1)])),
            (2, BTreeMap::from([("c".into(), -1)])),
        ]);
        assert_eq!(
            candidates(&ids, &r, Some("a"), true, 0),
            vec![("b".into(), 0.5)]
        );
        assert!(
            candidates(&ids, &r, None, false, 0)
                .iter()
                .all(|(id, _)| id != "c")
        );
        let mut s = ServerSettings {
            radio_mode: RadioMode::AutoJoin,
            ..Default::default()
        };
        assert!(s.validate(1).is_err());
        s.voice_channel = Some(2);
        assert!(s.validate(1).is_ok());
        s.player_locked = true;
        assert!(!s.permits_control(false, &[]));
        assert!(s.permits_control(true, &[]));
    }
}

pub mod id_set {
    use super::*;
    pub fn serialize<S: serde::Serializer>(ids: &BTreeSet<u64>, s: S) -> Result<S::Ok, S::Error> {
        ids.iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .serialize(s)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(d: D) -> Result<BTreeSet<u64>, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Id {
            Text(String),
            Number(u64),
        }
        Vec::<Id>::deserialize(d)?
            .into_iter()
            .map(|id| match id {
                Id::Text(v) => v.parse().map_err(serde::de::Error::custom),
                Id::Number(n) => Ok(n),
            })
            .collect()
    }
}
pub mod optional_id {
    use super::*;
    pub fn serialize<S: serde::Serializer>(id: &Option<u64>, s: S) -> Result<S::Ok, S::Error> {
        id.map(|i| i.to_string()).serialize(s)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Id {
            Text(String),
            Number(u64),
        }
        Option::<Id>::deserialize(d)?
            .map(|id| match id {
                Id::Text(v) => v.parse().map_err(serde::de::Error::custom),
                Id::Number(n) => Ok(n),
            })
            .transpose()
    }
}

#[cfg(test)]
mod persistence_tests {
    use super::*;
    #[test]
    fn imports_once_and_preserves_original_and_rejects_failed_writes() {
        let root = std::env::temp_dir().join(format!("music-import-{}", rand::random::<u64>()));
        std::fs::create_dir_all(&root).unwrap();
        let legacy = root.join("skip-settings.json");
        let bytes=br#"{"1":{"access":"moderators","mode":"majority","majority_scope":"eligible","moderator_roles":[22]}}"#;
        std::fs::write(&legacy, bytes).unwrap();
        let path = root.join("music.sqlite3");
        let mut store = Store::open(&path, &legacy).unwrap();
        assert_eq!(
            store.settings(1).unwrap().skip.mode,
            crate::skip::SkipMode::Majority
        );
        let mut changed = store.settings(1).unwrap();
        changed.volume = 41;
        store.save(1, changed).unwrap();
        drop(store);
        let mut store = Store::open(&path, &legacy).unwrap();
        assert_eq!(store.settings(1).unwrap().volume, 41);
        assert_eq!(std::fs::read(&legacy).unwrap(), bytes);
        assert_eq!(
            store.export_legacy().unwrap()["1"]["moderator_roles"][0],
            22
        );
        store.db.execute_batch("PRAGMA query_only=ON").unwrap();
        let mut changed = store.settings(1).unwrap();
        changed.volume = 90;
        assert!(store.save(1, changed).is_err());
        assert_eq!(store.settings(1).unwrap().volume, 41);
        drop(store);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn ids_remain_exact_and_bad_resources_are_rejected() {
        let id = 1524199248248176751;
        let s = ServerSettings {
            voice_channel: Some(id),
            dj_roles: BTreeSet::from([id]),
            ..Default::default()
        };
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json["voice_channel"], id.to_string());
        assert_eq!(json["dj_roles"][0], id.to_string());
        assert_eq!(serde_json::from_value::<ServerSettings>(json).unwrap(), s);
        assert!(
            s.validate_resources(
                1,
                &BTreeSet::new(),
                &BTreeSet::from([id]),
                &BTreeSet::new(),
                &BTreeSet::from(["lofi".into()])
            )
            .is_err()
        );
    }
    #[test]
    fn listeners_have_equal_weight_and_discovery_avoids_rated_favorites() {
        let ids = vec!["a".into(), "b".into(), "c".into(), "d".into()];
        let ratings = BTreeMap::from([
            (1, BTreeMap::from([("a".into(), 1), ("b".into(), 1)])),
            (2, BTreeMap::from([("c".into(), 1)])),
        ]);
        let weights: BTreeMap<_, _> = candidates(&ids, &ratings, None, true, 0)
            .into_iter()
            .collect();
        assert_eq!(weights["a"] + weights["b"], weights["c"]);
        assert_eq!(
            candidates(&ids, &ratings, None, true, 4),
            vec![("d".into(), 1.0)]
        );
        let all_disliked = BTreeMap::from([(1, ids.iter().map(|id| (id.clone(), -1)).collect())]);
        assert!(candidates(&ids, &all_disliked, None, true, 0).is_empty());
        assert_eq!(
            candidates(&["a".into()], &ratings, Some("a"), true, 0),
            vec![("a".into(), 1.0)]
        );
    }
    #[test]
    fn ratings_and_counts_are_scoped_and_deletion_is_separate() {
        let mut s =
            Store::open(Path::new(":memory:"), Path::new("nonexistent-import.json")).unwrap();
        s.rate(1, 2, "a", 1).unwrap();
        s.set_privacy(
            1,
            2,
            Privacy {
                enabled: true,
                visible: true,
            },
            false,
        )
        .unwrap();
        s.record_listening(1, "a", &BTreeSet::from([2]), 20)
            .unwrap();
        s.set_privacy(1, 2, Privacy::default(), false).unwrap();
        s.record_listening(1, "a", &BTreeSet::from([2]), 20)
            .unwrap();
        assert_eq!(s.privacy(1, 2).unwrap().1, 20);
        assert_eq!(s.privacy(9, 2).unwrap().1, 0);
        s.clear_ratings(1, 2).unwrap();
        assert_eq!(s.privacy(1, 2).unwrap().1, 20);
        s.set_privacy(1, 2, Privacy::default(), true).unwrap();
        assert_eq!(s.privacy(1, 2).unwrap().1, 0);
    }

    #[test]
    fn like_toggles_only_the_selected_person_server_and_recording() {
        let s = Store::open(Path::new(":memory:"), Path::new("nonexistent-import.json")).unwrap();
        s.rate(1, 2, "a", 1).unwrap();
        s.rate(2, 3, "a", 1).unwrap();
        s.rate(1, 3, "b", 1).unwrap();
        s.rate(1, 3, "a", -1).unwrap();
        assert!(s.toggle_like(1, 3, "a").unwrap());
        assert_eq!(s.ratings(1, &BTreeSet::from([3])).unwrap()[&3]["a"], 1);
        assert!(!s.toggle_like(1, 3, "a").unwrap());
        let ratings = s.ratings(1, &BTreeSet::from([2, 3])).unwrap();
        assert!(!ratings[&3].contains_key("a"));
        assert_eq!(ratings[&3]["b"], 1);
        assert_eq!(ratings[&2]["a"], 1);
        assert_eq!(s.ratings(2, &BTreeSet::from([3])).unwrap()[&3]["a"], 1);
        assert!(s.toggle_like(1, 3, "a").unwrap());
        for (g, u, track) in [(0, 3, "a"), (1, 0, "a"), (1, 3, "")] {
            assert!(s.toggle_like(g, u, track).is_err());
        }
    }
}
