//! The lock file, `.atama/claude-lock.json`: the IDs of the agents and the
//! environments that Atama created.
//!
//! An agent is a stored object of the API. The API documentation says to
//! create it once and use its ID for each session. The lock file keeps the
//! ID, the version and the body of each agent, by its name. On the next run,
//! [`AgentLock::ensure_agent`] uses the stored agent if its body is the same,
//! and updates it if the body changed. The environments work the same way,
//! but the API has no update of an environment in Atama, so a changed
//! environment is a new one.
//!
//! ```json
//! {
//!   "agents": {"<name>": {"id": "agent_...", "version": 3, "definition": {}}},
//!   "environments": {"<name>": {"id": "env_...", "definition": {}}}
//! }
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::api::{AgentRef, AgentsApi};

/// The path of the lock file, relative to the working directory.
pub const LOCK_PATH: &str = ".atama/claude-lock.json";

/// The status of a conflict: another program changed the agent.
const CONFLICT: u16 = 409;

/// The lock file of one working directory.
pub struct AgentLock {
    path: PathBuf,
    entries: Value,
}

impl AgentLock {
    /// Reads the lock file at `path`. A missing file is an empty lock.
    ///
    /// Fails if the file cannot be read, or it is not a JSON object.
    pub fn load(path: &Path) -> Result<AgentLock, String> {
        let entries = match fs::read_to_string(path) {
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => json!({}),
            Err(err) => {
                return Err(format!("cannot read lock file `{}`: {err}", path.display()));
            }
            Ok(text) => serde_json::from_str::<Value>(&text)
                .ok()
                .filter(Value::is_object)
                .ok_or_else(|| {
                    format!(
                        "lock file `{}` is not a JSON object, delete it to create new agents",
                        path.display()
                    )
                })?,
        };
        Ok(AgentLock {
            path: path.to_path_buf(),
            entries,
        })
    }

    /// The entries of the lock file, as JSON.
    pub fn entries(&self) -> &Value {
        &self.entries
    }

    /// Returns the ID and the version of the agent `name` with the body
    /// `body` (see [`super::definition::AgentDefinition::to_json`]).
    ///
    /// - No entry: creates the agent, and stores it.
    /// - An entry with the same body: uses it, and sends no request.
    /// - An entry with another body: updates the agent with the stored
    ///   version, and stores the new version.
    ///
    /// It writes the file after each change. Fails if a request fails, or
    /// the file cannot be written. A conflict (409) means that another
    /// program changed the agent.
    pub fn ensure_agent(
        &mut self,
        api: &mut dyn AgentsApi,
        name: &str,
        body: &Value,
    ) -> Result<AgentRef, String> {
        let entry = &self.entries["agents"][name];
        if let (Some(id), Some(version)) = (entry["id"].as_str(), entry["version"].as_u64()) {
            if entry["definition"] == *body {
                return Ok(AgentRef {
                    id: id.to_string(),
                    version,
                });
            }
            let id = id.to_string();
            let mut update = body.clone();
            update["version"] = json!(version);
            let agent = api.update_agent(&id, &update).map_err(|error| {
                if error.status == Some(CONFLICT) {
                    format!(
                        "agent `{name}` ({id}) changed since its version {version} in `{}`. \
                         Make sure that no other job uses the same `name=`. {error}",
                        self.path.display()
                    )
                } else {
                    error.message
                }
            })?;
            self.store("agents", name, &agent.id, Some(agent.version), body)?;
            return Ok(agent);
        }
        let agent = api.create_agent(body)?;
        self.store("agents", name, &agent.id, Some(agent.version), body)?;
        Ok(agent)
    }

    /// Returns the ID of the environment `name` with the body `body` (see
    /// [`super::definition::EnvironmentDefinition::to_json`]). It creates the
    /// environment if the lock file has no entry with the same body, and
    /// writes the file.
    pub fn ensure_environment(
        &mut self,
        api: &mut dyn AgentsApi,
        name: &str,
        body: &Value,
    ) -> Result<String, String> {
        let entry = &self.entries["environments"][name];
        if let Some(id) = entry["id"].as_str()
            && entry["definition"] == *body
        {
            return Ok(id.to_string());
        }
        let id = api.create_environment(body)?;
        self.store("environments", name, &id, None, body)?;
        Ok(id)
    }

    fn store(
        &mut self,
        group: &str,
        name: &str,
        id: &str,
        version: Option<u64>,
        body: &Value,
    ) -> Result<(), String> {
        let mut entry = json!({"id": id});
        if let Some(version) = version {
            entry["version"] = json!(version);
        }
        entry["definition"] = body.clone();
        if !self.entries[group].is_object() {
            self.entries[group] = json!({});
        }
        self.entries[group][name] = entry;
        self.save()
    }

    /// Writes the lock file, and makes its directory.
    pub fn save(&self) -> Result<(), String> {
        let error = |err: std::io::Error| {
            format!("cannot write lock file `{}`: {err}", self.path.display())
        };
        if let Some(dir) = self.path.parent()
            && !dir.as_os_str().is_empty()
        {
            fs::create_dir_all(dir).map_err(error)?;
        }
        let text = serde_json::to_string_pretty(&self.entries).unwrap_or_default();
        fs::write(&self.path, format!("{text}\n")).map_err(error)
    }
}
