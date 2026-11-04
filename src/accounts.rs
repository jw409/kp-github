//! Explicit account scope shared by stdio and HTTP. No mutable active account.
use crate::{
    auth,
    events::{EventsClient, EventsConfig},
    github::client::GithubClient,
};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::OnceCell;

#[derive(Clone, Default)]
pub(crate) struct EventSetup {
    pub client: Option<Arc<EventsClient>>,
    pub error: Option<String>,
}

impl EventSetup {
    fn load(
        result: Result<Option<EventsConfig>, crate::events::EventsError>,
        account: Option<&str>,
    ) -> Self {
        let result = result.and_then(|config| {
            config
                .map(|config| {
                    if account
                        .is_some_and(|name| !name.eq_ignore_ascii_case(&config.github_account))
                    {
                        return Err(crate::events::EventsError::Config(
                            "profile account differs from selected account",
                        ));
                    }
                    EventsClient::from_config(config)
                })
                .transpose()
        });
        match result {
            Ok(client) => Self {
                client: client.map(Arc::new),
                error: None,
            },
            Err(error) => {
                tracing::warn!("Developer events disabled: {error}");
                Self {
                    client: None,
                    error: Some(error.to_string()),
                }
            }
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    version: u32,
    accounts: Vec<AccountConfig>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AccountConfig {
    github_account: String,
    events_config: Option<PathBuf>,
}

pub(crate) struct Account {
    login: String,
    client: OnceCell<Arc<GithubClient>>,
    pub events: EventSetup,
}

pub(crate) struct Accounts {
    named: BTreeMap<String, Account>,
    default: Option<String>,
    legacy: Option<Arc<GithubClient>>,
    legacy_events: EventSetup,
}

impl Accounts {
    pub fn from_environment() -> anyhow::Result<Self> {
        let default = auth::selected_account()?.map(|name| name.to_ascii_lowercase());
        let Some(path) = std::env::var_os("KP_GITHUB_ACCOUNTS_CONFIG") else {
            let client = Arc::new(GithubClient::new(&auth::resolve_token()?));
            let events = EventSetup::load(EventsConfig::from_environment(), default.as_deref());
            let mut named = BTreeMap::new();
            if let Some(name) = &default {
                named.insert(
                    name.clone(),
                    Account {
                        login: name.clone(),
                        client: OnceCell::new_with(Some(client.clone())),
                        events: events.clone(),
                    },
                );
            }
            return Ok(Self {
                named,
                default,
                legacy: Some(client),
                legacy_events: events,
            });
        };
        // This file explicitly authorizes named credentials; never discover it.
        let path = Path::new(&path);
        anyhow::ensure!(
            std::fs::symlink_metadata(path)?.is_file(),
            "accounts config must be a regular file"
        );
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take(65537)
            .read_to_end(&mut bytes)?;
        anyhow::ensure!(bytes.len() <= 65536, "accounts config exceeds 64 KiB");
        let config: Config = serde_json::from_slice(&bytes).map_err(|_| {
            anyhow::anyhow!("invalid accounts config; expected version 1 accounts list")
        })?;
        if std::env::var_os("KP_GITHUB_EVENTS_CONFIG").is_some() {
            tracing::warn!("Multi-account mode uses each entry's events_config; KP_GITHUB_EVENTS_CONFIG is ignored");
        }
        Self::from_config(config, default)
    }

    fn from_config(config: Config, default: Option<String>) -> anyhow::Result<Self> {
        anyhow::ensure!(
            config.version == 1 && !config.accounts.is_empty() && config.accounts.len() <= 100,
            "accounts config must have version 1 and 1-100 accounts"
        );
        let mut named = BTreeMap::new();
        for entry in config.accounts {
            auth::validate_account(&entry.github_account)?;
            let name = entry.github_account.to_ascii_lowercase();
            anyhow::ensure!(
                !named.contains_key(&name),
                "duplicate account in accounts config"
            );
            let profile = match entry.events_config {
                None => Ok(None),
                Some(path) if !path.is_absolute() => Err(crate::events::EventsError::Config(
                    "events_config must be an absolute path",
                )),
                Some(path) => EventsConfig::from_path(&path).map(Some),
            };
            named.insert(
                name.clone(),
                Account {
                    login: entry.github_account,
                    client: OnceCell::new(),
                    events: EventSetup::load(profile, Some(&name)),
                },
            );
        }
        anyhow::ensure!(
            default.as_ref().is_none_or(|name| named.contains_key(name)),
            "KP_GITHUB_ACCOUNT must be listed in the accounts config"
        );
        Ok(Self {
            named,
            default,
            legacy: None,
            legacy_events: EventSetup::default(),
        })
    }

    fn select(&self, requested: Option<&str>) -> Result<Option<&Account>, String> {
        if let Some(name) = requested.or(self.default.as_deref()) {
            auth::validate_account(name).map_err(|e| e.to_string())?;
            return self.named.get(&name.to_ascii_lowercase()).map(Some)
                .ok_or_else(|| "GitHub account is not enabled for this server; configure KP_GITHUB_ACCOUNTS_CONFIG explicitly".into());
        }
        if self.legacy.is_some() {
            return Ok(None);
        }
        Err(
            "Specify github_account for this request or set an explicit KP_GITHUB_ACCOUNT default"
                .into(),
        )
    }

    pub async fn client(&self, requested: Option<&str>) -> Result<Arc<GithubClient>, String> {
        let Some(account) = self.select(requested)? else {
            return Ok(self.legacy.as_ref().unwrap().clone());
        };
        let name = account.login.clone();
        // One cached client per account, including concurrent first requests.
        // Failed lookups are retryable; never fall back to another identity.
        account
            .client
            .get_or_try_init(|| async {
                let token = tokio::task::spawn_blocking(move || auth::named_token(&name))
                    .await
                    .map_err(|_| "GitHub credential lookup failed".to_string())?
                    .map_err(|error| error.to_string())?;
                Ok(Arc::new(GithubClient::new(&token)))
            })
            .await
            .cloned()
    }

    pub fn events(&self, requested: Option<&str>) -> Result<EventSetup, String> {
        Ok(match self.select(requested)? {
            Some(account) => account.events.clone(),
            None => self.legacy_events.clone(),
        })
    }

    pub fn names(&self) -> Vec<String> {
        self.named.keys().cloned().collect()
    }
    pub fn requires_selector(&self) -> bool {
        self.legacy.is_none() && self.default.is_none()
    }

    #[cfg(test)]
    pub(crate) fn with_clients(entries: Vec<(&str, GithubClient, EventSetup)>) -> Self {
        Self {
            named: entries
                .into_iter()
                .map(|(name, client, events)| {
                    (
                        name.into(),
                        Account {
                            login: name.into(),
                            client: OnceCell::new_with(Some(Arc::new(client))),
                            events,
                        },
                    )
                })
                .collect(),
            default: None,
            legacy: None,
            legacy_events: EventSetup::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config(names: &[&str]) -> Config {
        Config {
            version: 1,
            accounts: names
                .iter()
                .map(|name| AccountConfig {
                    github_account: (*name).into(),
                    events_config: None,
                })
                .collect(),
        }
    }
    #[test]
    fn explicit_default_is_required_and_unlisted_accounts_cannot_be_selected() {
        let accounts = Accounts::from_config(config(&["work-dev", "other-dev"]), None).unwrap();
        assert!(accounts.select(None).is_err());
        assert!(accounts.select(Some("WORK-dev")).is_ok());
        assert!(accounts.select(Some("unlisted")).is_err());
        assert!(Accounts::from_config(config(&["work-dev"]), Some("other-dev".into())).is_err());
        let accounts =
            Accounts::from_config(config(&["work-dev", "other-dev"]), Some("work-dev".into()))
                .unwrap();
        assert_eq!(accounts.select(None).unwrap().unwrap().login, "work-dev");
        assert_eq!(
            accounts.select(Some("other-dev")).unwrap().unwrap().login,
            "other-dev"
        );
        assert_eq!(accounts.select(None).unwrap().unwrap().login, "work-dev");
    }
    #[test]
    fn invalid_and_duplicate_configuration_never_exposes_account_values() {
        for names in [
            vec!["work-dev", "WORK-dev"],
            vec!["ghp_private_marker"],
            vec![],
        ] {
            assert!(Accounts::from_config(config(&names), None).is_err());
        }
    }
    #[test]
    fn broken_and_mismatched_profiles_disable_only_events() {
        let mut input = config(&["work-dev", "other-dev"]);
        input.accounts[0].events_config = Some(PathBuf::from("relative-is-invalid.json"));
        let accounts = Accounts::from_config(input, None).unwrap();
        assert!(accounts.events(Some("work-dev")).unwrap().error.is_some());
        assert!(accounts.events(Some("other-dev")).unwrap().error.is_none());
        let profile: EventsConfig = serde_json::from_value(serde_json::json!({
            "version":1,"endpoint":"https://events.invalid","audience":"fixture",
            "allowed_orgs":["FirstOrg","SecondOrg"],"github_account":"other-dev"
        }))
        .unwrap();
        assert!(EventSetup::load(Ok(Some(profile)), Some("work-dev"))
            .error
            .unwrap()
            .contains("differs"));
    }
}
