//! Tests of the lock file in `src/agent_loop_claude/lock.rs`, with a mock
//! `AgentsApi`. They do not call the real API.

use std::fs;
use std::path::PathBuf;

use serde_json::{Value, json};

use atama::agent_loop_claude::api::{AgentRef, AgentsApi, ApiError, EventStream};
use atama::agent_loop_claude::events::SessionEvent;
use atama::agent_loop_claude::lock::{AgentLock, LOCK_PATH};

/// An API that records each request, and answers from its fields.
#[derive(Default)]
struct MockApi {
    requests: Vec<(String, Value)>,
    next_version: u64,
    update_error: Option<ApiError>,
}

impl AgentsApi for MockApi {
    fn create_agent(&mut self, body: &Value) -> Result<AgentRef, ApiError> {
        self.requests.push(("create_agent".into(), body.clone()));
        self.next_version = 1;
        Ok(AgentRef {
            id: format!("agent_{}", self.requests.len()),
            version: 1,
        })
    }

    fn update_agent(&mut self, id: &str, body: &Value) -> Result<AgentRef, ApiError> {
        self.requests
            .push((format!("update_agent {id}"), body.clone()));
        if let Some(error) = self.update_error.clone() {
            return Err(error);
        }
        Ok(AgentRef {
            id: id.to_string(),
            version: body["version"].as_u64().unwrap() + 1,
        })
    }

    fn get_agent(&mut self, _id: &str) -> Result<Value, ApiError> {
        unimplemented!()
    }

    fn create_environment(&mut self, body: &Value) -> Result<String, ApiError> {
        self.requests
            .push(("create_environment".into(), body.clone()));
        Ok(format!("env_{}", self.requests.len()))
    }

    fn create_session(&mut self, _body: &Value) -> Result<String, ApiError> {
        unimplemented!()
    }

    fn get_session(&mut self, _id: &str) -> Result<Value, ApiError> {
        unimplemented!()
    }

    fn send_events(&mut self, _session_id: &str, _events: &[Value]) -> Result<(), ApiError> {
        unimplemented!()
    }

    fn list_events(&mut self, _session_id: &str) -> Result<Vec<SessionEvent>, ApiError> {
        unimplemented!()
    }

    fn open_stream(&mut self, _session_id: &str) -> Result<Box<dyn EventStream>, ApiError> {
        unimplemented!()
    }
}

/// A new, empty directory for one test.
fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("atama-lock-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn the_lock_file_is_in_the_atama_directory() {
    assert_eq!(LOCK_PATH, ".atama/claude-lock.json");
}

#[test]
fn creates_reuses_and_updates_an_agent() {
    let dir = temp_dir("agent");
    let path = dir.join(".atama/claude-lock.json");
    let mut api = MockApi::default();
    let body = json!({"name": "a", "model": "claude-opus-5-5", "tools": []});

    // No entry: create, and write the file with its directory.
    let mut lock = AgentLock::load(&path).unwrap();
    assert_eq!(lock.entries(), &json!({}));
    let agent = lock.ensure_agent(&mut api, "a", &body).unwrap();
    assert_eq!(
        agent,
        AgentRef {
            id: "agent_1".into(),
            version: 1
        }
    );
    assert_eq!(api.requests, [("create_agent".to_string(), body.clone())]);

    // The same body, in a new run: no request.
    let mut lock = AgentLock::load(&path).unwrap();
    assert_eq!(lock.ensure_agent(&mut api, "a", &body).unwrap(), agent);
    assert_eq!(api.requests.len(), 1);

    // Another body: update with the stored version.
    let changed =
        json!({"name": "a", "model": "claude-opus-5-5", "system": "Be brief.", "tools": []});
    let updated = lock.ensure_agent(&mut api, "a", &changed).unwrap();
    assert_eq!(updated.version, 2);
    let mut expected = changed.clone();
    expected["version"] = json!(1);
    assert_eq!(
        api.requests[1],
        ("update_agent agent_1".to_string(), expected)
    );

    let stored: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        stored["agents"]["a"],
        json!({"id": "agent_1", "version": 2, "definition": changed})
    );
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_conflict_names_the_agent_and_the_lock_file() {
    let dir = temp_dir("conflict");
    let path = dir.join("lock.json");
    fs::write(
        &path,
        json!({"agents": {"a": {"id": "agent_9", "version": 3, "definition": {}}}}).to_string(),
    )
    .unwrap();
    let mut lock = AgentLock::load(&path).unwrap();
    let mut api = MockApi {
        update_error: Some(ApiError {
            status: Some(409),
            message: "`x` responded with 409: version mismatch".into(),
        }),
        ..MockApi::default()
    };
    assert_eq!(
        lock.ensure_agent(&mut api, "a", &json!({"name": "a"}))
            .unwrap_err(),
        format!(
            "agent `a` (agent_9) changed since its version 3 in `{}`. Make sure that no other \
             job uses the same `name=`. `x` responded with 409: version mismatch",
            path.display()
        )
    );
    api.update_error = Some(ApiError {
        status: Some(500),
        message: "server broke".into(),
    });
    assert_eq!(
        lock.ensure_agent(&mut api, "a", &json!({"name": "a"}))
            .unwrap_err(),
        "server broke"
    );
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn creates_an_environment_for_each_new_definition() {
    let dir = temp_dir("environment");
    let path = dir.join("lock.json");
    // A file whose groups are not objects still works.
    fs::write(&path, r#"{"environments": 1}"#).unwrap();
    let mut lock = AgentLock::load(&path).unwrap();
    let mut api = MockApi::default();
    let body = json!({"name": "atama-default", "config": {"type": "cloud"}});
    assert_eq!(
        lock.ensure_environment(&mut api, "atama-default", &body)
            .unwrap(),
        "env_1"
    );
    assert_eq!(
        lock.ensure_environment(&mut api, "atama-default", &body)
            .unwrap(),
        "env_1"
    );
    let changed = json!({"name": "atama-default", "config": {"type": "cloud", "x": 1}});
    assert_eq!(
        lock.ensure_environment(&mut api, "atama-default", &changed)
            .unwrap(),
        "env_2"
    );
    assert_eq!(api.requests.len(), 2);
    let stored: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        stored["environments"]["atama-default"],
        json!({"id": "env_2", "definition": changed})
    );
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn rejects_a_lock_file_that_is_not_an_object() {
    let dir = temp_dir("invalid");
    for text in ["not json", "[1, 2]"] {
        let path = dir.join("lock.json");
        fs::write(&path, text).unwrap();
        assert_eq!(
            AgentLock::load(&path).err().unwrap(),
            format!(
                "lock file `{}` is not a JSON object, delete it to create new agents",
                path.display()
            )
        );
    }
    // A directory cannot be read as a file.
    let error = AgentLock::load(&dir).err().unwrap();
    assert!(
        error.starts_with(&format!("cannot read lock file `{}`:", dir.display())),
        "{error}"
    );
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn fails_if_the_lock_file_cannot_be_written() {
    let dir = temp_dir("unwritable");
    let blocker = dir.join("blocker");
    let path = blocker.join("lock.json");
    let mut lock = AgentLock::load(&path).unwrap();
    // Now the parent of the lock file is a file, so the directory cannot be
    // made.
    fs::write(&blocker, "").unwrap();
    let error = lock
        .ensure_agent(&mut MockApi::default(), "a", &json!({}))
        .unwrap_err();
    assert!(
        error.starts_with(&format!("cannot write lock file `{}`:", path.display())),
        "{error}"
    );
    // A lock file without a directory part is written in the working directory.
    AgentLock::load(&dir.join("x.json"))
        .unwrap()
        .save()
        .unwrap();
    fs::remove_dir_all(&dir).unwrap();
}
