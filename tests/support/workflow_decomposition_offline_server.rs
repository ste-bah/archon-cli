use std::collections::BTreeMap;
use std::io::Read;
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

#[derive(Clone, Debug, Default)]
pub struct Counts(pub Arc<Mutex<BTreeMap<String, usize>>>);

impl Counts {
    pub fn snapshot(&self) -> BTreeMap<String, usize> {
        self.0.lock().unwrap().clone()
    }
}

#[derive(Clone, Debug)]
pub struct RequestRecord {
    pub method: String,
    pub path: String,
    pub auth_present: bool,
    pub known_path: bool,
    pub role: String,
    pub prompt: String,
    pub answer: String,
}

#[derive(Clone, Debug, Default)]
pub struct Requests(pub Arc<Mutex<Vec<RequestRecord>>>);

impl Requests {
    pub fn snapshot(&self) -> Vec<RequestRecord> {
        self.0.lock().unwrap().clone()
    }
}

pub struct FakeProvider {
    pub base_url: String,
    pub counts: Counts,
    pub requests: Requests,
    blocked_role: Option<String>,
    release: Arc<(Mutex<bool>, std::sync::Condvar)>,
    entered: Arc<(Mutex<bool>, std::sync::Condvar)>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl FakeProvider {
    pub fn start() -> Self {
        Self::start_inner(None)
    }

    pub fn start_blocked(role: &str) -> Self {
        Self::start_inner(Some(role.to_string()))
    }

    fn start_inner(blocked_role: Option<String>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let counts = Counts::default();
        let request_counts = counts.clone();
        let requests = Requests::default();
        let request_log = requests.clone();
        let release = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
        let entered = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
        let thread_release = release.clone();
        let thread_entered = entered.clone();
        let thread_blocked_role = blocked_role.clone();
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let thread_stop = stop.clone();
        let thread = thread::spawn(move || {
            while !thread_stop.load(std::sync::atomic::Ordering::SeqCst) {
                let Ok((mut stream, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                };
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let mut request = Vec::new();
                let mut buf = [0u8; 8192];
                loop {
                    match stream.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            request.extend_from_slice(&buf[..n]);
                            if request.windows(4).any(|w| w == b"\r\n\r\n") {
                                let head_end =
                                    request.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
                                let head = String::from_utf8_lossy(&request[..head_end]);
                                let length = head
                                    .lines()
                                    .find_map(|line| {
                                        let (name, value) = line.split_once(':')?;
                                        name.eq_ignore_ascii_case("content-length")
                                            .then(|| value.trim().parse::<usize>().ok())
                                            .flatten()
                                    })
                                    .unwrap_or(0);
                                while request.len() < head_end + length {
                                    match stream.read(&mut buf) {
                                        Ok(0) | Err(_) => break,
                                        Ok(n) => request.extend_from_slice(&buf[..n]),
                                    }
                                }
                                break;
                            }
                        }
                    }
                }
                if request.is_empty() {
                    continue;
                }
                let body_start = request
                    .windows(4)
                    .position(|w| w == b"\r\n\r\n")
                    .map(|i| i + 4)
                    .unwrap_or(request.len());
                let head = String::from_utf8_lossy(&request[..body_start]);
                let request_line = head.lines().next().unwrap_or_default();
                let mut parts = request_line.split_whitespace();
                let method = parts.next().unwrap_or_default().to_string();
                let path = parts.next().unwrap_or_default().to_string();
                let headers = head.lines().skip(1).filter_map(|line| line.split_once(':'));
                let mut auth_present = false;
                let mut auth_valid = false;
                for (name, value) in headers {
                    if name.eq_ignore_ascii_case("x-api-key") {
                        auth_present = true;
                        auth_valid = value.trim() == "offline-test-key";
                    } else if name.eq_ignore_ascii_case("authorization") {
                        auth_present = true;
                        auth_valid = value.trim() == "Bearer offline-test-key";
                    }
                }
                let known_path = method == "POST" && path == "/v1/messages";
                let body: serde_json::Value =
                    serde_json::from_slice(&request[body_start..]).unwrap_or_default();
                let prompt = body.to_string();
                let (role, answer) = scripted_answer(&prompt);
                request_log.0.lock().unwrap().push(RequestRecord {
                    method,
                    path,
                    auth_present,
                    known_path,
                    role: role.to_string(),
                    prompt: prompt.clone(),
                    answer: answer.clone(),
                });
                *request_counts
                    .0
                    .lock()
                    .unwrap()
                    .entry(role.to_string())
                    .or_default() += 1;
                if thread_blocked_role.as_deref() == Some(role) {
                    *thread_entered.0.lock().unwrap() = true;
                    thread_entered.1.notify_all();
                    let mut released = thread_release.0.lock().unwrap();
                    while !*released {
                        released = thread_release.1.wait(released).unwrap();
                    }
                }
                let escaped = serde_json::to_string(&answer).unwrap();
                let (status, content_type, response) = if !known_path {
                    (
                        "404 Not Found",
                        "application/json",
                        "{\"error\":\"unknown path\"}".to_string(),
                    )
                } else if !auth_valid {
                    (
                        "401 Unauthorized",
                        "application/json",
                        "{\"error\":\"invalid test key\"}".to_string(),
                    )
                } else if body["stream"] == false {
                    (
                        "200 OK",
                        "application/json",
                        serde_json::json!({
                            "id": "msg-local",
                            "type": "message",
                            "role": "assistant",
                            "content": [{"type": "text", "text": answer}],
                            "model": "offline",
                            "stop_reason": "end_turn",
                            "stop_sequence": null,
                            "usage": {"input_tokens": 1, "output_tokens": 1}
                        })
                        .to_string(),
                    )
                } else {
                    (
                        "200 OK",
                        "text/event-stream",
                        format!(
                            "event: message_start\ndata: {{\"type\":\"message_start\",\"message\":{{\"id\":\"msg-local\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"offline\",\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{{\"input_tokens\":1,\"output_tokens\":1}}}}}}\n\nevent: content_block_start\ndata: {{\"type\":\"content_block_start\",\"index\":0,\"content_block\":{{\"type\":\"text\",\"text\":\"\"}}}}\n\nevent: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"text_delta\",\"text\":{escaped}}}}}\n\nevent: content_block_stop\ndata: {{\"type\":\"content_block_stop\",\"index\":0}}\n\nevent: message_delta\ndata: {{\"type\":\"message_delta\",\"delta\":{{\"stop_reason\":\"end_turn\",\"stop_sequence\":null}},\"usage\":{{\"output_tokens\":1}}}}\n\nevent: message_stop\ndata: {{\"type\":\"message_stop\"}}\n\n"
                        ),
                    )
                };
                use std::io::Write;
                let _ = write!(
                    stream,
                    "HTTP/1.1 {}\r\nContent-Type: {}\r\nCache-Control: no-cache\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    status,
                    content_type,
                    response.len(),
                    response
                );
                let _ = stream.flush();
            }
        });
        Self {
            base_url: format!("http://{address}/v1/messages"),
            counts,
            requests,
            blocked_role,
            release,
            entered,
            stop,
            thread: Some(thread),
        }
    }

    pub fn wait_until_blocked(&self, wait_interval: Duration) -> bool {
        assert!(self.blocked_role.is_some(), "provider has no blocked role");
        let mut entered = self.entered.0.lock().unwrap();
        while !*entered {
            let (next, timeout) = self.entered.1.wait_timeout(entered, wait_interval).unwrap();
            entered = next;
            if timeout.timed_out() && !*entered {
                return false;
            }
        }
        true
    }

    pub fn release(&self) {
        *self.release.0.lock().unwrap() = true;
        self.release.1.notify_all();
    }
}

impl Drop for FakeProvider {
    fn drop(&mut self) {
        eprintln!(
            "fake provider received {} requests",
            self.requests.snapshot().len()
        );
        self.release();
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        let _ = std::net::TcpStream::connect(
            self.base_url
                .trim_start_matches("http://")
                .split('/')
                .next()
                .unwrap(),
        );
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn scripted_answer(prompt: &str) -> (&'static str, String) {
    if prompt.contains("Adversarially judge every acceptance check") || prompt.contains("decisions")
    {
        let ids = prompt
            .split('"')
            .filter(|part| part.starts_with("AC-") || part.starts_with("SUP-"))
            .collect::<std::collections::BTreeSet<_>>();
        let decisions = ids
            .into_iter()
            .map(|id| {
                serde_json::json!({
                    "id": id,
                    "verdict": "accepted",
                    "counterexample": "",
                    "reason": "The command directly checks the declared outcome and exits non-zero when it is false."
                })
            })
            .collect::<Vec<_>>();
        return (
            "judge",
            serde_json::json!({"decisions": decisions}).to_string(),
        );
    }
    if prompt.contains("Author exactly one acceptance entry") {
        let id = prompt
            .split("Author ONLY entry ")
            .nth(1)
            .and_then(|s| s.split(':').next())
            .unwrap_or("AC-001");
        if id.starts_with("SUP-REQ-X-001") {
            return (
                "acceptance",
                format!(
                    r#"{{"id":"{id}","criterion":"The first artifact contains the required line.","check":{{"kind":"command","command":"test -f src/alpha.txt && grep -qx 'alpha ready' src/alpha.txt","cwd":"repo_root"}},"gap_permitted":false,"covers":["REQ-X-001"],"judgment":{{"verdict":"accepted","counterexample":"","reason":"","host_call_id":""}}}}"#
                ),
            );
        }
        if id.starts_with("SUP-REQ-X-002") {
            return (
                "acceptance",
                format!(
                    r#"{{"id":"{id}","criterion":"The second artifact is ready JSON.","check":{{"kind":"command","command":"python3 -c 'import json,sys; assert json.load(open(sys.argv[1]))[\"ready\"] is True' src/beta.json","cwd":"repo_root"}},"gap_permitted":false,"covers":["REQ-X-002"],"judgment":{{"verdict":"accepted","counterexample":"","reason":"","host_call_id":""}}}}"#
                ),
            );
        }
        return (
            "acceptance",
            format!(
                r#"{{"id":"{id}","criterion":"The repository records a JSON outcome artifact at `.archon/proof/outcome.json` with the boolean field `ready` set to true.","check":{{"kind":"command","command":"python3 -c 'import json,sys; assert json.load(open(sys.argv[1]))[\"ready\"] is True' .archon/proof/outcome.json","cwd":"repo_root"}},"gap_permitted":false,"covers":[],"judgment":{{"verdict":"accepted","counterexample":"","reason":"","host_call_id":""}}}}"#
            ),
        );
    }
    if prompt.contains("Author one complete task-skeleton JSON artifact") {
        return ("skeleton", r#"{"schema_version":1,"acceptance_digest":"","tasks":[{"task_id":"TASK-X-010","file_name":"TASK-X-010.md","depends_on":[],"blocks":["TASK-X-020"],"implements":["REQ-X-001"],"deliverable_contracts":[{"kind":"alpha_text","artifact_path":"src/alpha.txt","artifact_format":"text","min_instances":1},{"kind":"generic_outcome","artifact_path":".archon/proof/outcome.json","artifact_format":"json","required_true_fields":["ready"],"min_instances":1}]},{"task_id":"TASK-X-020","file_name":"TASK-X-020.md","depends_on":[{"task_id":"TASK-X-010","consumes":[{"artifact_path":"src/alpha.txt"}],"ordering_only":false}],"blocks":[],"implements":["REQ-X-002"],"deliverable_contracts":[{"kind":"beta_json","artifact_path":"src/beta.json","artifact_format":"json","required_true_fields":["ready"],"min_instances":1}]}]}"#.into());
    }
    if prompt.contains("Author the complete TASK body") {
        let second = prompt.contains("task_id TASK-X-020");
        let (id, title, dep, blocks, req, path, text, contracts) = if second {
            (
                "TASK-X-020",
                "Create beta data",
                "\n  - task_id: TASK-X-010\n    consumes:\n      - artifact_path: src/alpha.txt\n    ordering_only: false",
                "[]",
                "REQ-X-002",
                "src/beta.json",
                "Write a JSON object with ready set to true.",
                "- kind: beta_json\n    artifact_path: src/beta.json\n    artifact_format: json\n    required_true_fields: [ready]\n    min_instances: 1",
            )
        } else {
            (
                "TASK-X-010",
                "Create alpha data",
                "[]",
                "[TASK-X-020]",
                "REQ-X-001",
                "src/alpha.txt",
                "Write the nonempty line alpha ready and create the JSON outcome artifact with ready set to true.",
                "- kind: alpha_text\n    artifact_path: src/alpha.txt\n    artifact_format: text\n    min_instances: 1\n  - kind: generic_outcome\n    artifact_path: .archon/proof/outcome.json\n    artifact_format: json\n    required_true_fields: [ready]\n    min_instances: 1",
            )
        };
        let observations = if second {
            format!("`{path}` — absent")
        } else {
            format!("`{path}` — absent\n`.archon/proof/outcome.json` — absent")
        };
        let focused_tests = if second {
            format!(
                "- `python3 -c 'import json,sys; assert json.load(open(sys.argv[1]))[\"ready\"] is True' {path}`"
            )
        } else {
            format!(
                "- `test -s {path}`\n- `python3 -c 'import json,sys; assert json.load(open(sys.argv[1]))[\"ready\"] is True' .archon/proof/outcome.json`"
            )
        };
        return (
            "body",
            format!(
                "```yaml\ntask_id: {id}\ntitle: {title}\ncomplexity: low\nstatus: ready\ndepends_on: {dep}\nblocks: {blocks}\nimplements: [{req}]\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts:\n  {contracts}\n```\n\n## Objective\n{text}\n\n## Files Expected to Change\n{observations}\n\n## Focused Tests\n{focused_tests}\n"
            ),
        );
    }
    if prompt.contains("audit") || prompt.contains("Audit") {
        return ("audit", r#"{"verdict":"pass","findings":[]}"#.into());
    }
    (
        "unexpected",
        r#"{"error":"unscripted fake provider request"}"#.into(),
    )
}

#[cfg(test)]
mod tests {
    use super::scripted_answer;

    #[test]
    fn routes_current_acceptance_author_prompt_and_extracts_its_id() {
        let prompt = r#"{"task":"Author exactly one acceptance entry identified below. Author ONLY entry AC-X-001: The outcome is ready."}"#;
        let (role, answer) = scripted_answer(prompt);
        assert_eq!(role, "acceptance");
        assert!(answer.contains(r#""id":"AC-X-001""#), "{answer}");
    }

    #[test]
    fn supplementary_answer_covers_the_requirement_named_by_the_id() {
        let prompt = r#"{"task":"Author exactly one acceptance entry identified below. Author ONLY entry SUP-REQ-X-001: The first artifact is ready."}"#;
        let (role, answer) = scripted_answer(prompt);
        assert_eq!(role, "acceptance");
        assert!(answer.contains(r#""covers":["REQ-X-001"]"#), "{answer}");
        assert!(answer.contains("src/alpha.txt"), "{answer}");
    }

    #[test]
    fn skeleton_and_body_answers_use_the_fixture_requirement_ids() {
        let (_, skeleton) = scripted_answer("Author one complete task-skeleton JSON artifact");
        assert!(skeleton.contains("REQ-X-001"), "{skeleton}");
        assert!(skeleton.contains("REQ-X-002"), "{skeleton}");
        let (_, body) = scripted_answer("Author the complete TASK body");
        assert!(body.contains("REQ-X-001"), "{body}");
    }

    #[test]
    fn golden_answers_make_the_outcome_check_executable_and_owned() {
        let prompt = r#"{"task":"Author exactly one acceptance entry identified below. Author ONLY entry AC-X-001: The repository records a JSON outcome artifact at `.archon/proof/outcome.json` with the boolean field `ready` set to true."}"#;
        let (_, answer) = scripted_answer(prompt);
        let entry: serde_json::Value = serde_json::from_str(&answer).unwrap();
        assert_eq!(entry["check"]["kind"], "command");
        assert_eq!(entry["check"]["cwd"], "repo_root");
        assert!(
            entry["check"]["command"]
                .as_str()
                .unwrap()
                .contains("[\"ready\"] is True"),
            "{entry}"
        );

        let (_, skeleton) = scripted_answer("Author one complete task-skeleton JSON artifact");
        let skeleton: serde_json::Value = serde_json::from_str(&skeleton).unwrap();
        let contracts = skeleton["tasks"][0]["deliverable_contracts"]
            .as_array()
            .unwrap();
        assert!(
            contracts
                .iter()
                .any(|contract| contract["artifact_path"] == ".archon/proof/outcome.json"),
            "{skeleton}"
        );

        let prompt = "Author the complete TASK body for host-frozen task_id TASK-X-010.";
        let (_, body) = scripted_answer(prompt);
        assert!(
            body.contains("`.archon/proof/outcome.json` — absent"),
            "{body}"
        );
        assert!(body.contains("json.load(open(sys.argv[1]))"), "{body}");
        let yaml = body
            .split("```yaml\n")
            .nth(1)
            .unwrap()
            .split("\n```")
            .next()
            .unwrap();
        let body_yaml: serde_yaml_ng::Value = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(
            body_yaml["deliverable_contracts"]
                .as_sequence()
                .unwrap()
                .len(),
            2
        );

        let (_, judge) = scripted_answer(
            r#"{"task":"Adversarially judge every acceptance check. decisions include ","id":"AC-X-001"}"#,
        );
        let judge: serde_json::Value = serde_json::from_str(&judge).unwrap();
        assert_eq!(judge["decisions"][0]["verdict"], "accepted");
        assert_eq!(judge["decisions"][0]["counterexample"], "");
    }
}
