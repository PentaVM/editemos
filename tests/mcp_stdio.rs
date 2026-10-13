//! The real binary speaking MCP over real pipes. Anything other than JSON on
//! stdout would break every client, so every line is parsed.

mod common;

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

struct Server {
    child: std::process::Child,
    out: BufReader<std::process::ChildStdout>,
    next: u64,
}

impl Server {
    fn start(home: &std::path::Path) -> Server {
        let mut child = Command::new(env!("CARGO_BIN_EXE_editemos"))
            .arg("serve")
            .env("EDITEMOS_HOME", home)
            .env_remove("EDITEMOS_PROJECT")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn editemos serve");
        let out = BufReader::new(child.stdout.take().unwrap());
        Server { child, out, next: 0 }
    }

    fn send(&mut self, v: Value) {
        let stdin = self.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{v}").unwrap();
        stdin.flush().unwrap();
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        let id = self.next;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        let mut line = String::new();
        self.out.read_line(&mut line).expect("read reply");
        let v: Value = serde_json::from_str(line.trim()).unwrap_or_else(|e| panic!("stdout carried a non-JSON line {line:?}: {e}"));
        assert_eq!(v["jsonrpc"], "2.0");
        assert_eq!(v["id"], id);
        v
    }

    fn call(&mut self, name: &str, args: Value) -> Value {
        let r = self.request("tools/call", json!({"name": name, "arguments": args}));
        assert!(r.get("error").is_none(), "protocol error: {r}");
        r["result"].clone()
    }

    fn text(result: &Value) -> String {
        result["content"][0]["text"].as_str().unwrap_or("").to_string()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.stdin.take();
        let _ = self.child.wait();
    }
}

#[test]
fn full_session_over_stdio() {
    let home = common::temp_home("mcp");
    {
        let mut s = Server::start(&home);

        let init = s.request("initialize", json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "0"}}));
        assert_eq!(init["result"]["serverInfo"]["name"], "editemos");
        assert!(init["result"]["capabilities"]["tools"].is_object());
        s.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));

        let tools = s.request("tools/list", json!({}));
        let names: Vec<String> = tools["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect();
        for n in ["editemos_context", "editemos_memory_add", "editemos_extract", "editemos_motion", "editemos_kdenlive", "editemos_skills"] {
            assert!(names.iter().any(|x| x == n), "missing tool {n}: {names:?}");
        }

        let r = s.call("editemos_project", json!({"action": "create", "name": "Canal Sur", "activate": true}));
        assert_eq!(r["isError"], false, "{r}");
        let r = s.call("editemos_memory_add", json!({"text": "Cortes siempre sobre la palabra, nunca en el silencio", "kind": "preference", "confidence": "high"}));
        assert_eq!(r["isError"], false, "{r}");

        let ctx = s.call("editemos_context", json!({"task": "cortes"}));
        let md = Server::text(&ctx);
        assert!(md.contains("Canal Sur") && md.contains("sobre la palabra"), "{md}");

        let scene = s.call("editemos_motion", json!({"action": "new", "kind": "title", "text": "Hola"}));
        assert_eq!(scene["isError"], false, "{scene}");
        let v: Value = serde_json::from_str(&Server::text(&scene)).unwrap();
        assert!(std::path::Path::new(v["scene"].as_str().unwrap()).exists());

        // errors are tool errors, not protocol errors, and the server keeps going
        let bad = s.call("editemos_motion", json!({"action": "new", "kind": "nonsense"}));
        assert_eq!(bad["isError"], true);
        let again = s.request("ping", json!({}));
        assert!(again["result"].is_object());

        let res = s.request("resources/list", json!({}));
        assert!(res["result"]["resources"].as_array().unwrap().iter().any(|r| r["uri"].as_str().unwrap().ends_with("/context")));
        let prompt = s.request("prompts/get", json!({"name": "editemos", "arguments": {"task": "cortar la entrevista"}}));
        assert!(prompt["result"]["messages"][0]["content"]["text"].as_str().unwrap().contains("Canal Sur"));
    }
    let _ = std::fs::remove_dir_all(&home);
}
