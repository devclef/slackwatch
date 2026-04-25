use config::{Config, ConfigError, Environment, File};
use serde_derive::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Clone)]
#[allow(unused)]
pub struct Settings {
    #[serde(default)]
    pub system: System,
    pub notifications: Option<Notifications>,
    pub gitops: Option<Vec<GitopsConfig>>,
}


impl Default for System {
    fn default() -> Self {
        System {
            schedule: default_schedule(),
            data_dir: default_data_dir(),
            run_at_startup: default_run_at_startup(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[allow(unused)]
pub struct System {
    #[serde(default = "default_schedule")]
    pub schedule: String,
    #[serde(default = "default_data_dir")]
    pub data_dir: String,
    #[serde(default = "default_run_at_startup")]
    pub run_at_startup: bool,
}

fn default_schedule() -> String {
    "0 0 */2 * * *".to_string()
}
fn default_data_dir() -> String {
    "/app/slackwatch/data".to_string()
}

fn default_run_at_startup() -> bool {
    false
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[allow(unused)]
pub struct GitopsConfig {
    pub name: String,
    pub repository_url: String,
    pub branch: String,
    pub commit_name: String,
    pub commit_email: String,
    pub access_token_env_name: String,
    pub commit_message: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[allow(unused)]
pub struct Notifications {
    pub ntfy: Option<Ntfy>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[allow(unused)]
pub struct Ntfy {
    pub url: String,
    pub topic: String,
    pub reminder: String,
    pub token: String,
    #[serde(default)]
    pub callback_url: Option<String>,
    #[serde(default = "default_auto_rescan_delay")]
    pub auto_rescan_delay: String,
}

fn default_auto_rescan_delay() -> String {
    "5m".to_string()
}

impl Settings {
    pub fn new() -> Result<Self, ConfigError> {
        //get config from env var
        let env_config = std::env::var("SLACKWATCH_CONFIG").unwrap_or("".to_string());
        let s = Config::builder()
            //add source if env var slackwatch_config path is set
            .add_source(File::with_name("/app/config/config").required(false))
//            .add_source(File::with_name(".env.toml").required(false))
            .add_source(File::with_name(".env.yaml").required(false))
            .add_source(File::with_name(&env_config).required(false))
            .add_source(Environment::with_prefix("slackwatch"))
            .build()?;
        //print config
        println!("{:?}", s);
        s.try_deserialize::<Settings>().or_else(|e| Err(e))
    }
    //add clone
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn test_settings_load_success() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let mut file = File::create(&config_path).unwrap();
        writeln!(
            file,
            r#"
            [system]
            schedule = "0 0 * * * *"
            data_dir = "/tmp/data"

            [[gitops]]
            name = "example-repo"
            repository_url = "https://example.com/repo.git"
            branch = "main"
            commit_name = "Example Committer"
            commit_email = "committer@example.com"
            access_token_env_name = "GITOPS_TOKEN"
            commit_message = "Update"

            [notifications.ntfy]
            url = "http://ntfy.example.com"
            topic = "updates"
            reminder = "24h"
            token = "secrettoken"
            "#
        )
        .unwrap();

        std::env::set_var("SLACKWATCH_CONFIG", config_path.to_str().unwrap());
        std::env::remove_var("SLACKWATCH_SYSTEM.SCHEDULE");
        let settings = Settings::new().expect("Settings should load successfully");
        //remove conflicting env var ones for now
        assert_eq!(settings.system.data_dir, "/tmp/data");
//        assert_eq!(settings.gitops[0].name, "example-repo");
    }

    #[test]
    fn test_environment_override() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let mut file = File::create(&config_path).unwrap();
        writeln!(
            file,
            r#"
            [system]
            schedule = "0 0 * * *"
            data_dir = "/tmp/data"
            "#
        )
        .unwrap();

        std::env::set_var("SLACKWATCH_CONFIG", config_path.to_str().unwrap());
        // Override the schedule using an environment variable
        std::env::set_var("SLACKWATCH_SYSTEM.SCHEDULE", "0 12 * * * *");
        std::env::set_var("SLACKWATCH_SYSTEM.DATA_DIR", "/tmp/data");
        std::env::set_var("SLACKWATCH_GITOPS_0_NAME", "example-repo");
        std::env::set_var(
            "SLACKWATCH_NOTIFICATIONS.NTFY.URL",
            "http://ntfy.example.com",
        );
        //Has to be . not _ for nested
        std::env::set_var("SLACKWATCH_NOTIFICATIONS.NTFY.TOPIC", "updates");
        std::env::set_var("SLACKWATCH_NOTIFICATIONS.NTFY.REMINDER", "24h");
        std::env::set_var("SLACKWATCH_NOTIFICATIONS.NTFY.TOKEN", "secrettoken");

        let settings =
            Settings::new().expect("Settings should load successfully with env override");
        assert_eq!(
            settings.system.schedule, "0 12 * * * *",
            "Environment variable should override file configuration"
        );
    }

    #[test]
    fn test_ntfy_config_with_new_fields() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let mut file = File::create(&config_path).unwrap();
        writeln!(
            file,
            r#"
            [system]
            schedule = "0 0 * * * *"
            data_dir = "/tmp/data"

            [notifications.ntfy]
            url = "http://ntfy.example.com"
            topic = "updates"
            reminder = "24h"
            token = "secrettoken"
            callback_url = "http://slackwatch:8080"
            auto_rescan_delay = "10m"
            "#
        )
        .unwrap();

        std::env::set_var("SLACKWATCH_CONFIG", config_path.to_str().unwrap());
        let settings = Settings::new().expect("Settings should load successfully");
        let ntfy = settings.notifications.unwrap().ntfy.unwrap();
        assert_eq!(ntfy.callback_url, Some("http://slackwatch:8080".to_string()));
        assert_eq!(ntfy.auto_rescan_delay, "10m");
    }

    #[test]
    fn test_ntfy_config_default_auto_rescan_delay() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let mut file = File::create(&config_path).unwrap();
        writeln!(
            file,
            r#"
            [system]
            schedule = "0 0 * * * *"
            data_dir = "/tmp/data"

            [notifications.ntfy]
            url = "http://ntfy.example.com"
            topic = "updates"
            reminder = "24h"
            token = "secrettoken"
            "#
        )
        .unwrap();

        std::env::set_var("SLACKWATCH_CONFIG", config_path.to_str().unwrap());
        let settings = Settings::new().expect("Settings should load successfully");
        let ntfy = settings.notifications.unwrap().ntfy.unwrap();
        assert_eq!(ntfy.auto_rescan_delay, "5m");
    }
}
