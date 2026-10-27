use crate::Error;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::PathBuf,
};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkipAccess {
    #[default]
    Everyone,
    Moderators,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkipMode {
    #[default]
    Instant,
    Majority,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MajorityScope {
    #[default]
    Call,
    Eligible,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SkipSettings {
    pub access: SkipAccess,
    pub mode: SkipMode,
    pub majority_scope: MajorityScope,
    pub moderator_roles: BTreeSet<u64>,
}

impl SkipSettings {
    pub fn allows(&self, manager: bool, moderator: bool, roles: &[u64]) -> bool {
        self.access == SkipAccess::Everyone
            || manager
            || if self.moderator_roles.is_empty() {
                moderator
            } else {
                roles.iter().any(|id| self.moderator_roles.contains(id))
            }
    }

    pub fn required_votes(&self, humans: usize, eligible: usize) -> usize {
        match self.mode {
            SkipMode::Instant => 1,
            SkipMode::Majority => match self.majority_scope {
                MajorityScope::Call => humans / 2 + 1,
                MajorityScope::Eligible => eligible / 2 + 1,
            },
        }
    }

    pub fn summary(&self) -> String {
        let access = match self.access {
            SkipAccess::Everyone => "Everyone in the voice channel",
            SkipAccess::Moderators => "Moderators only (server managers are always eligible)",
        };
        let mode = match (self.mode, self.majority_scope) {
            (SkipMode::Instant, _) => "Instant: one eligible member is enough",
            (SkipMode::Majority, MajorityScope::Call) => {
                "Majority of all humans in the voice channel"
            }
            (SkipMode::Majority, MajorityScope::Eligible) => {
                "Majority of eligible humans in the voice channel"
            }
        };
        let roles = if self.moderator_roles.is_empty() {
            "None: use server moderation permissions".to_owned()
        } else {
            let mut roles = self
                .moderator_roles
                .iter()
                .take(10)
                .map(|id| format!("<@&{id}>"))
                .collect::<Vec<_>>()
                .join(", ");
            if self.moderator_roles.len() > 10 {
                roles.push_str(" (additional roles omitted)");
            }
            roles
        };
        format!("Access: {access}\nMode: {mode}\nModerator roles: {roles}")
    }
}

pub struct SettingsStore {
    path: PathBuf,
    guilds: BTreeMap<u64, SkipSettings>,
}

impl SettingsStore {
    pub fn load(path: PathBuf) -> Result<Self, Error> {
        let guilds = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(error) => return Err(error.into()),
        };
        let store = Self { path, guilds };
        for (&guild, settings) in &store.guilds {
            Self::validate(guild, settings)?;
        }
        Ok(store)
    }

    fn validate(guild: u64, settings: &SkipSettings) -> Result<(), Error> {
        if guild == 0
            || settings.moderator_roles.contains(&0)
            || settings.moderator_roles.contains(&guild)
        {
            return Err("Invalid guild or moderator role in skip settings.".into());
        }
        Ok(())
    }

    pub fn get(&self, guild: u64) -> SkipSettings {
        self.guilds.get(&guild).cloned().unwrap_or_default()
    }

    pub fn set(&mut self, guild: u64, settings: SkipSettings) -> Result<(), Error> {
        Self::validate(guild, &settings)?;
        let mut updated = self.guilds.clone();
        updated.insert(guild, settings);
        let parent = self.path.parent().ok_or("Invalid settings path.")?;
        fs::create_dir_all(parent)?;
        let pending = self.path.with_extension("pending");
        let mut file = fs::File::create(&pending)?;
        file.write_all(&serde_json::to_vec_pretty(&updated)?)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&pending, &self.path)?;
        self.guilds = updated;
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct SkipVotes {
    generation: usize,
    pub voters: BTreeSet<u64>,
    pub required: usize,
    requests: BTreeMap<u64, std::time::Instant>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct VoteProgress {
    pub votes: usize,
    pub required: usize,
    pub added: bool,
}

impl VoteProgress {
    pub fn passed(&self) -> bool {
        self.votes >= self.required
    }
}

impl SkipVotes {
    pub fn reset(&mut self, generation: usize) {
        self.generation = generation;
        self.voters.clear();
        self.required = 0;
        self.requests.clear();
    }

    pub fn begin_request(
        &mut self,
        voter: u64,
        now: std::time::Instant,
        present: &BTreeSet<u64>,
    ) -> Result<(), Error> {
        self.requests.retain(|id, _| present.contains(id));
        if self.requests.get(&voter).is_some_and(|last| {
            now.saturating_duration_since(*last) < std::time::Duration::from_secs(2)
        }) {
            return Err("Please wait two seconds before voting again.".into());
        }
        self.requests.insert(voter, now);
        Ok(())
    }

    pub fn cast(
        &mut self,
        generation: usize,
        voter: u64,
        eligible: &BTreeSet<u64>,
        required: usize,
    ) -> Result<VoteProgress, Error> {
        if !eligible.contains(&voter) || required == 0 {
            return Err("You are not eligible to skip this track.".into());
        }
        if self.generation != generation {
            self.reset(generation);
        }
        self.voters.retain(|id| eligible.contains(id));
        let added = self.voters.insert(voter);
        self.required = required;
        Ok(VoteProgress {
            votes: self.voters.len(),
            required,
            added,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_both_majority_scopes() {
        let mut policy = SkipSettings::default();
        assert!(policy.allows(false, false, &[]));
        assert_eq!(policy.required_votes(10, 2), 1);
        policy.mode = SkipMode::Majority;
        assert_eq!(policy.required_votes(10, 2), 6);
        assert_eq!(policy.required_votes(3, 3), 2);
        assert_eq!(policy.required_votes(1, 1), 1);
        policy.majority_scope = MajorityScope::Eligible;
        assert_eq!(policy.required_votes(10, 2), 2);
    }

    #[test]
    fn custom_roles_and_server_managers() {
        let mut policy = SkipSettings {
            access: SkipAccess::Moderators,
            ..Default::default()
        };
        assert!(!policy.allows(false, false, &[]));
        assert!(policy.allows(false, true, &[]));
        policy.moderator_roles.insert(22);
        assert!(!policy.allows(false, true, &[23]));
        assert!(policy.allows(false, false, &[22]));
        assert!(policy.allows(true, false, &[]));
    }

    #[test]
    fn duplicate_departed_and_previous_track_votes_do_not_count() {
        let mut votes = SkipVotes::default();
        let humans = BTreeSet::from([1, 2, 3]);
        assert!(!votes.cast(1, 1, &humans, 2).unwrap().passed());
        assert_eq!(
            votes.cast(1, 1, &humans, 2).unwrap(),
            VoteProgress {
                votes: 1,
                required: 2,
                added: false
            }
        );
        assert!(votes.cast(1, 2, &humans, 2).unwrap().passed());
        assert_eq!(
            votes.cast(1, 3, &BTreeSet::from([2, 3]), 2).unwrap().votes,
            2
        );
        assert_eq!(votes.cast(2, 3, &humans, 2).unwrap().votes, 1);
        assert!(votes.cast(2, 9, &humans, 2).is_err());
        votes.reset(2);
        assert!(votes.voters.is_empty());
    }

    #[test]
    fn repeated_vote_requests_are_throttled_without_delaying_a_new_track() {
        let mut votes = SkipVotes::default();
        let now = std::time::Instant::now();
        let present = BTreeSet::from([1, 2]);
        assert!(votes.begin_request(1, now, &present).is_ok());
        assert!(
            votes
                .begin_request(1, now + std::time::Duration::from_secs(1), &present)
                .is_err()
        );
        assert!(votes.begin_request(2, now, &present).is_ok());
        assert!(
            votes
                .begin_request(1, now + std::time::Duration::from_secs(2), &present)
                .is_ok()
        );
        votes.reset(1);
        assert!(votes.begin_request(1, now, &present).is_ok());
    }

    #[test]
    fn settings_survive_restart_and_corruption_fails_closed() {
        let root = std::env::temp_dir().join(format!(
            "vozen-skip-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = root.join("settings.json");
        let mut store = SettingsStore::load(path.clone()).unwrap();
        let policy = SkipSettings {
            access: SkipAccess::Moderators,
            mode: SkipMode::Majority,
            majority_scope: MajorityScope::Eligible,
            moderator_roles: BTreeSet::from([22, 33]),
        };
        store.set(11, policy.clone()).unwrap();
        store.set(12, SkipSettings::default()).unwrap();
        assert_eq!(SettingsStore::load(path.clone()).unwrap().get(11), policy);
        assert_eq!(store.get(13), SkipSettings::default());
        let invalid = SkipSettings {
            moderator_roles: BTreeSet::from([11]),
            ..Default::default()
        };
        assert!(store.set(11, invalid).is_err());
        assert_eq!(store.get(11), policy);
        let original_path = store.path.clone();
        fs::write(root.join("blocked"), b"not a directory").unwrap();
        store.path = root.join("blocked/settings.json");
        assert!(store.set(11, SkipSettings::default()).is_err());
        assert_eq!(store.get(11), policy);
        store.path = original_path;
        fs::write(&path, b"broken JSON").unwrap();
        assert!(SettingsStore::load(path).is_err());
        assert!(root.starts_with(std::env::temp_dir()));
        fs::remove_dir_all(root).unwrap();
    }
}
