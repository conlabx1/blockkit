use serde_json::{Value, json};
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};

const LOCKED_FILES: [&str; 5] = ["check", "rust-toolchain.toml", "rust-toolchain", ".claude/settings.json", ".agents/hooks.json"];
const LOCKED_DIRS: [&str; 3] = ["tools/check", ".cargo", ".codex"];
const HOLDERS: [&str; 4] = ["tools", ".claude", ".agents", ""];
const BUILT: &str = "tools/check/target";
const WRITERS: [&str; 25] = [
    "rm", "rmdir", "unlink", "mv", "cp", "ln", "tee", "sed", "gsed", "perl", "dd", "chmod", "chown", "chflags", "xattr", "truncate", "touch", "install",
    "rsync", "ditto", "patch", "shred", "mkdir", "curl", "wget",
];
const GIT_WRITERS: [&str; 6] = ["rm", "mv", "checkout", "restore", "apply", "clean"];
const PREFIXES: [&str; 16] = ["sudo", "env", "xargs", "command", "exec", "nohup", "time", "nice", "then", "do", "else", "elif", "if", "while", "until", "!"];
const SHELLS: [&str; 6] = ["sh", "bash", "zsh", "dash", "ksh", "eval"];
const INTERPRETERS: [&str; 10] = ["python", "python3", "node", "ruby", "perl", "osascript", "swift", "php", "awk", "gawk"];
const PATH_KEYS: [&str; 6] = ["path", "file", "target", "dir", "dest", "uri"];
const COMMAND_KEYS: [&str; 3] = ["command", "cmd", "script"];

fn locked(rel: &str) -> bool {
    LOCKED_FILES.contains(&rel) || LOCKED_DIRS.iter().any(|dir| rel == *dir || rel.starts_with(&format!("{dir}/"))) && !(rel == BUILT || rel.starts_with(&format!("{BUILT}/")))
}

fn holds_locked(rel: &str) -> bool {
    HOLDERS.contains(&rel)
}

fn glob(pattern: &[u8], text: &[u8], start: bool) -> bool {
    let hidden = start && text.first() == Some(&b'.');
    match (pattern.first(), text.first()) {
        (None, None) => true,
        (Some(b'*'), _) => glob(&pattern[1..], text, start) || text.first().is_some_and(|letter| *letter != b'/') && !hidden && glob(pattern, &text[1..], false),
        (Some(b'?'), Some(letter)) if *letter != b'/' && !hidden => glob(&pattern[1..], &text[1..], false),
        (Some(want), Some(letter)) if want == letter => glob(&pattern[1..], &text[1..], *letter == b'/'),
        _ => false,
    }
}

fn hit(rel: &str) -> Option<String> {
    if rel.contains(['*', '?']) {
        let fixed = rel.split('/').take_while(|part| !part.contains(['*', '?'])).collect::<Vec<_>>().join("/");
        if locked(&fixed) {
            return Some(fixed);
        }
        let inside = |item: &&&str| {
            let head = rel.split('/').take(item.split('/').count()).collect::<Vec<_>>().join("/");
            head.contains(['*', '?']) && glob(head.as_bytes(), item.as_bytes(), true)
        };
        return LOCKED_FILES.iter().chain(LOCKED_DIRS.iter()).chain(HOLDERS.iter()).find(|item| !item.is_empty() && glob(rel.as_bytes(), item.as_bytes(), true)).or_else(|| LOCKED_DIRS.iter().find(inside)).map(|item| item.to_string());
    }
    (locked(rel) || holds_locked(rel)).then(|| rel.to_string())
}

fn without_heredocs(text: &str) -> String {
    let mut out = String::new();
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        out.push_str(line);
        out.push('\n');
        let mut ends = Vec::new();
        let mut rest = line;
        while let Some(at) = rest.find("<<") {
            rest = &rest[at + 2..];
            if let Some(after) = rest.strip_prefix('<') {
                rest = after;
                continue;
            }
            let end: String = rest.trim_start_matches('-').trim_start().chars().take_while(|letter| !letter.is_whitespace() && !";|&<>()".contains(*letter)).collect();
            let end = end.trim_matches(['\'', '"', '\\']).to_string();
            if !end.is_empty() {
                ends.push(end);
            }
        }
        for end in ends {
            if lines.by_ref().any(|body| body.trim() == end) {
                out.push_str(&end);
                out.push('\n');
            }
        }
    }
    out
}

fn segments(text: &str) -> Vec<Vec<(String, bool)>> {
    let mut all = vec![Vec::new()];
    let mut word = String::new();
    let mut quoted = false;
    let mut quote = None;
    let mut previous = ' ';
    let mut escaped = false;
    for letter in text.chars() {
        let after_redirect = previous == '>';
        previous = letter;
        if escaped {
            escaped = false;
            if letter != '\n' {
                word.push(letter);
            }
            continue;
        }
        let segment = all.last_mut().expect("segments always holds one");
        match quote {
            Some(open) if letter == open => quote = None,
            Some('"') if letter == '\\' => escaped = true,
            Some(_) => word.push(letter),
            None if letter == '\\' => escaped = true,
            None if letter == '\'' || letter == '"' => {
                quote = Some(letter);
                quoted = true;
            }
            None if after_redirect && (letter == '|' || letter == '&') => {}
            None if ";|&()\n`".contains(letter) => {
                if !word.is_empty() {
                    segment.push((std::mem::take(&mut word), std::mem::take(&mut quoted)));
                }
                all.push(Vec::new());
            }
            None if letter == '>' || letter == '<' || letter.is_whitespace() => {
                if !word.is_empty() {
                    segment.push((std::mem::take(&mut word), std::mem::take(&mut quoted)));
                }
                if !letter.is_whitespace() {
                    segment.push((letter.to_string(), false));
                }
            }
            None => word.push(letter),
        }
    }
    if !word.is_empty() {
        all.last_mut().expect("segments always holds one").push((word, quoted));
    }
    all
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

fn identity(path: &Path) -> Option<(u64, u64)> {
    fs::metadata(path).ok().map(|meta| (meta.dev(), meta.ino()))
}

pub struct Guard {
    roots: Vec<PathBuf>,
    root_id: Option<(u64, u64)>,
    home: PathBuf,
}

impl Guard {
    pub fn new(root: &Path, home: &Path) -> Guard {
        let mut roots = vec![normalize(root)];
        if let Ok(real) = root.canonicalize()
            && !roots.contains(&real)
        {
            roots.push(real);
        }
        Guard { roots, root_id: identity(root), home: home.to_path_buf() }
    }

    fn expand(&self, word: &str) -> PathBuf {
        match word.strip_prefix('~') {
            Some(rest) if rest.is_empty() || rest.starts_with('/') => self.home.join(rest.trim_start_matches('/')),
            _ => PathBuf::from(word),
        }
    }

    fn inside(&self, full: &Path) -> Option<String> {
        let lower = |rel: &Path| rel.to_string_lossy().to_lowercase().trim_end_matches('/').to_string();
        if let Some(rel) = self.roots.iter().find_map(|root| full.strip_prefix(root).ok()) {
            return Some(lower(rel));
        }
        let root_id = self.root_id?;
        let mut ancestor = full;
        while let Some(parent) = ancestor.parent() {
            if identity(ancestor) == Some(root_id) {
                return full.strip_prefix(ancestor).ok().map(lower);
            }
            ancestor = parent;
        }
        None
    }

    fn relatives(&self, word: &str, bases: &[PathBuf]) -> Vec<String> {
        let word = word.trim();
        if word.is_empty() {
            return Vec::new();
        }
        let expanded = self.expand(word);
        let bases = if bases.is_empty() { &self.roots[..] } else { bases };
        let fulls: Vec<PathBuf> = if expanded.is_absolute() { vec![normalize(&expanded)] } else { bases.iter().map(|base| normalize(&base.join(&expanded))).collect() };
        fulls.iter().filter_map(|full| self.inside(full)).collect()
    }

    fn with_roots(&self, bases: &[PathBuf]) -> Vec<PathBuf> {
        bases.iter().chain(self.roots.iter()).cloned().collect()
    }

    fn writes_to(&self, word: &str, bases: &[PathBuf]) -> Option<String> {
        self.relatives(word, bases).iter().find_map(|rel| hit(rel))
    }

    fn mentions(&self, word: &str, bases: &[PathBuf], loose: bool) -> Option<String> {
        if !loose && !word.contains('/') {
            return None;
        }
        self.relatives(word, bases).into_iter().find(|rel| locked(rel) && (loose || rel != "check"))
    }

    fn shell_write(&self, word: &str, bases: &[PathBuf], fallback: bool) -> Option<String> {
        self.writes_to(word, bases).or_else(|| if fallback { self.relatives(word, &self.roots).into_iter().find(|rel| locked(rel)) } else { None })
    }

    fn shell_read(&self, word: &str, bases: &[PathBuf], fallback: bool, loose: bool) -> Option<String> {
        word.split(|letter: char| letter.is_whitespace() || "()'\";".contains(letter))
            .find_map(|piece| self.mentions(piece, bases, loose).or_else(|| if fallback { self.mentions(piece, &self.roots, loose) } else { None }))
    }

    fn command(&self, text: &str, bases: &[PathBuf]) -> Option<String> {
        let mut bases = if bases.is_empty() { self.roots.clone() } else { bases.to_vec() };
        let mut fallback = true;
        for segment in segments(&without_heredocs(text)) {
            let mut program: Option<String> = None;
            let mut git_write = false;
            let mut git_sub = false;
            let mut worktree = false;
            let mut redirect = false;
            let mut moved = None;
            for (word, quoted) in segment {
                if word == ">" {
                    redirect = true;
                    continue;
                }
                if word == "<" {
                    continue;
                }
                let parts: Vec<&str> = word.split(['=', ',', '{', '}']).filter(|part| !part.is_empty()).collect();
                let assignment = program.is_none() && word.split_once('=').is_some_and(|(name, _)| !name.is_empty() && name.chars().all(|letter| letter.is_ascii_alphanumeric() || letter == '_'));
                let prefixed = program.as_deref().is_some_and(|name| PREFIXES.contains(&name)) && !word.starts_with('-') && !word.contains('=');
                let first = (program.is_none() && !assignment) || prefixed;
                let name = program.as_deref().unwrap_or_default();
                if name == "cd" && !first && moved.is_none() {
                    moved = Some(bases.iter().map(|base| normalize(&base.join(self.expand(&word)))).collect::<Vec<_>>());
                }
                if name == "git" && GIT_WRITERS.contains(&word.as_str()) {
                    git_write = true;
                    git_sub = true;
                }
                if name == "git" && ["--staged", "-S", "--cached"].contains(&word.as_str()) && !worktree {
                    git_write = false;
                }
                if name == "git" && git_sub && ["--worktree", "-W"].contains(&word.as_str()) {
                    git_write = true;
                    worktree = true;
                }
                if SHELLS.contains(&name) && !first && !word.starts_with('-')
                    && let Some(found) = self.command(&word, &bases)
                {
                    return Some(found);
                }
                let writes = redirect || (!first && !assignment && (WRITERS.contains(&name) || git_write));
                let interpreter = INTERPRETERS.contains(&name);
                let read = !quoted || interpreter;
                if first {
                    program = parts.first().map(|part| part.rsplit('/').next().unwrap_or(part).to_string());
                }
                redirect = false;
                for part in parts.iter().skip(usize::from(assignment)) {
                    let found = if writes {
                        self.shell_write(part, &bases, fallback)
                    } else if read {
                        self.shell_read(part, &bases, fallback, interpreter)
                    } else {
                        None
                    };
                    if found.is_some() {
                        return found;
                    }
                }
            }
            if let Some(moved) = moved {
                bases = moved;
                fallback = false;
            }
        }
        None
    }

    fn patch(&self, text: &str, bases: &[PathBuf]) -> Option<String> {
        let bases = self.with_roots(bases);
        text.lines()
            .filter_map(|line| ["*** Add File:", "*** Update File:", "*** Delete File:", "*** Move to:"].iter().find_map(|head| line.strip_prefix(head)))
            .find_map(|path| self.writes_to(path, &bases))
    }

    fn path(&self, text: &str, bases: &[PathBuf]) -> Option<String> {
        self.writes_to(text, &self.with_roots(bases))
    }

    fn anything(&self, value: &Value, key: &str, bases: &[PathBuf]) -> Option<String> {
        let key = key.to_lowercase();
        match value {
            Value::String(text) if COMMAND_KEYS.iter().any(|name| key.starts_with(name) || key.ends_with(name)) => self.command(text, bases).or_else(|| self.patch(text, bases)),
            Value::String(text) if PATH_KEYS.iter().any(|name| key.starts_with(name) || key.ends_with(name)) => {
                self.relatives(text, &self.with_roots(bases)).into_iter().find(|rel| locked(rel) || rel.contains(['*', '?']) && hit(rel).is_some())
            }
            Value::String(text) => self.patch(text, bases),
            Value::Array(items) => items.iter().find_map(|item| self.anything(item, &key, bases)),
            Value::Object(fields) => fields.iter().find_map(|(name, item)| self.anything(item, name, bases)),
            _ => None,
        }
    }

    fn text(value: &Value) -> String {
        match value {
            Value::Array(items) => items.iter().map(|item| item.as_str().unwrap_or_default()).collect::<Vec<_>>().join(" "),
            other => other.as_str().unwrap_or_default().to_string(),
        }
    }

    pub fn judge(&self, harness: &str, data: &Value) -> Option<String> {
        let base = |value: &Value| value.as_str().filter(|text| !text.is_empty()).map(PathBuf::from);
        let (name, input, bases): (&str, &Value, Vec<PathBuf>) = match harness {
            "codex" => (data["tool_name"].as_str().unwrap_or_default(), &data["tool_input"], base(&data["cwd"]).into_iter().collect()),
            "hermes" => (data["tool_name"].as_str().unwrap_or_default(), &data["tool_input"], base(&data["tool_input"]["workdir"]).into_iter().chain(base(&data["cwd"])).collect()),
            "antigravity" => {
                let args = &data["toolCall"]["args"];
                (data["toolCall"]["name"].as_str().unwrap_or_default(), args, base(&args["Cwd"]).into_iter().chain(data["workspacePaths"].as_array().into_iter().flatten().filter_map(base)).collect())
            }
            _ => return None,
        };
        let named = |key: &str| Some(Self::text(&input[key])).filter(|text| !text.is_empty());
        let found = match (harness, name) {
            ("codex", "Bash") => named("command").map(|text| self.command(&text, &bases).or_else(|| self.patch(&text, &bases))),
            ("codex", "apply_patch") => named("command").map(|text| self.patch(&text, &bases)),
            ("hermes", "write_file") => named("path").map(|text| self.path(&text, &bases)),
            ("hermes", "patch") => Some(named("path").and_then(|text| self.path(&text, &bases)).or_else(|| named("patch").and_then(|text| self.patch(&text, &bases)))),
            ("hermes", "terminal") => named("command").map(|text| self.command(&text, &bases)),
            ("antigravity", "write_to_file" | "replace_file_content" | "multi_replace_file_content") => named("TargetFile").map(|text| self.path(&text, &bases)),
            ("antigravity", "run_command") => named("CommandLine").map(|text| self.command(&text, &bases)),
            _ => None,
        };
        found.unwrap_or_else(|| self.anything(input, "", &bases))
    }
}

pub fn reply(harness: &str, path: &str) -> String {
    let path = if path.is_empty() { "프로젝트 맨 위 폴더" } else { path };
    let reason = format!("blockkit 의 틀 파일({path})은 이 폴더에서 고치거나 열지 않습니다. 점검은 ./check 로만 돌리고, 틀을 바꿔야 하면 주인에게 알리십시오.");
    match harness {
        "codex" => json!({"hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision": "deny", "permissionDecisionReason": reason}}),
        "antigravity" => json!({"decision": "deny", "reason": reason}),
        _ => json!({"decision": "block", "reason": reason}),
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guard() -> Guard {
        Guard::new(Path::new("/p/my project"), Path::new("/u"))
    }

    fn codex_bash(command: &str) -> Value {
        json!({"tool_name": "Bash", "cwd": "/p/my project", "tool_input": {"command": command}})
    }

    #[test]
    fn 틀_파일을_바꾸는_셸_명령은_막는다() {
        for command in [
            "echo x > check",
            "echo x >> ./tools/check/src/gates.rs",
            "sed -i '' s/a/b/ tools/check/src/scan.rs",
            "cp other check",
            "rm -rf tools",
            "mv .claude/settings.json /tmp/x",
            "dd if=/dev/zero of=check",
            "python3 -c \"open('tools/check/x.rs','w')\"",
            "cd sub && rm ../check",
            "git checkout -- check",
            "chflags nouchg check",
            "cat x | tee rust-toolchain.toml",
            "mkdir .cargo",
            "touch .codex/config.toml",
            "printf x > \"/p/my project/.agents/hooks.json\"",
            "ECHO=1 cp a rust-toolchain",
            "sudo rm check",
            "find . -name x | xargs rm -f check",
            "env A=1 tee check",
            "cd tools && rm -rf check",
            "cd blocks && cp x ../rust-toolchain.toml",
            "rm -rf .",
            "git checkout .",
            "bash -lc \"echo x > check\"",
            "sh -c 'rm -rf tools/check/src'",
            "bash -c \"sed -i '' s/a/b/ rust-toolchain.toml\"",
            "eval 'rm check'",
            "rm -rf *",
            "rm -rf tools/*",
            "sed -i '' s/1.90/1.91/ *.toml",
            "git checkout HEAD~1 -- '*.toml'",
            "if [ -f x ]; then rm check; fi",
            "for f in a b; do cp $f check; done",
            "curl -o rust-toolchain.toml https://example.com/x",
            "echo x >| check",
            "echo x >& check",
            "apply_patch <<'EOF'\n*** Begin Patch\n*** Update File: check\n*** End Patch\nEOF",
            "cat tools/check/src/main.rs",
            "cargo run --manifest-path=tools/check/Cargo.toml",
            "rm -rf tools/check/src/*",
            "rm tools/che*/src/main.rs",
            "rm tools/che\\\nck/src/main.rs",
            "git restore --staged --worktree check",
            "git restore --worktree --staged check",
            "git restore -W -S check",
            "rm -rf .c*",
            "python3 -c \"open('check','w').write('x')\"",
            "python3 -c \"open('rust-toolchain.toml','w').write('x')\"",
            "node -e \"require('fs').mkdirSync('.cargo')\"",
            "rm /p/my\\ project/check",
            "cp x \\\n  check",
        ] {
            assert!(guard().judge("codex", &codex_bash(command)).is_some(), "{command}");
        }
        let inside = |command: &str| json!({"tool_name": "Bash", "cwd": "/p/my project/tools", "tool_input": {"command": command}});
        assert!(guard().judge("codex", &inside("rm -rf check/src/*")).is_some());
        assert!(guard().judge("codex", &inside("sed -i '' s/a/b/ check/src/*.rs")).is_some());
    }

    #[test]
    fn 흔한_명령과_점검_실행은_막지_않는다() {
        for command in [
            "./check",
            "./check --offline",
            "sh ./check --hook",
            "PATH=\"/opt/homebrew/opt/rustup/bin:$PATH\" ./check",
            "cargo check",
            "cargo test -p greet",
            "git commit -m \"fix check\"",
            "echo \"dev tools\"",
            "ls tools",
            "cat check",
            "rm -rf tools/check/target",
            "rm -rf tools/check/target/*",
            "git commit -m \"fix \\\"check\\\" (rm check)\"",
            "cp blocks/greet/src/lib.rs /tmp/x",
            "~/.cargo/bin/cargo build",
            "git status",
            "rm /tmp/check",
            "cd blocks && rm -rf greet/target",
            "git add . && git commit -m 'rm check'",
            "sed -n 1,5p blocks/greet/src/lib.rs",
            "git commit -m \"greet 정리 (tools/check 는 그대로)\"",
            "git commit -m \"첫 줄\n\n- .claude/settings.json 은 그대로\"",
            "cd blocks/greet && cp ../hello/Cargo.toml .",
            "cd /tmp && touch check",
            "cd blocks && mkdir check",
            "cat > blocks/greet/src/lib.rs <<'EOF'\nfn run() -> Check {\n    x => check\n}\n> check 를 돌린다\nEOF\ncargo test",
            "cargo test 2>&1 | tail -5",
            "sh ./check --offline",
            "git restore --staged .",
            "git restore -S blocks",
            "rm blocks/greet/notes\\ old.md",
            "rm -rf */target",
            "rm */*.orig",
            "touch */.gitkeep",
            "cp */Cargo.toml /tmp/x",
            "rm -rf ?cargo",
            "git grep -W foo .",
            "git diff -W .",
        ] {
            assert_eq!(guard().judge("codex", &codex_bash(command)), None, "{command}");
        }
    }

    #[test]
    fn 글롭의_별표와_물음표는_맨_앞이나_빗금_바로_뒤의_점과_맞지_않는다() {
        for (pattern, text) in [("*", "check"), (".c*", ".cargo"), ("*.rs", "a.b.rs"), ("a/.*", "a/.x"), ("a/b?c", "a/b.c")] {
            assert!(glob(pattern.as_bytes(), text.as_bytes(), true), "{pattern} {text}");
        }
        for (pattern, text) in [("*", ".cargo"), ("?cargo", ".cargo"), ("a/*", "a/.x"), ("a/?x", "a/.x")] {
            assert!(!glob(pattern.as_bytes(), text.as_bytes(), true), "{pattern} {text}");
        }
    }

    #[test]
    fn 패치와_파일_도구는_틀_파일을_고치지_못한다() {
        let patch = |path: &str| format!("*** Begin Patch\n*** Update File: {path}\n@@\n-a\n+b\n*** End Patch\n");
        let codex = |path: &str| json!({"tool_name": "apply_patch", "cwd": "/p/my project", "tool_input": {"command": patch(path)}});
        assert!(guard().judge("codex", &codex("tools/check/src/gates.rs")).is_some());
        assert!(guard().judge("codex", &codex("Tools/Check/src/gates.rs")).is_some());
        assert_eq!(guard().judge("codex", &codex("blocks/greet/src/lib.rs")), None);
        let moved = json!({"tool_name": "apply_patch", "cwd": "/p/my project", "tool_input": {"command": "*** Update File: a.rs\n*** Move to: check\n"}});
        assert!(guard().judge("codex", &moved).is_some());
        let hermes = |tool: &str, input: Value| json!({"tool_name": tool, "cwd": "/elsewhere", "tool_input": input});
        assert!(guard().judge("hermes", &hermes("write_file", json!({"path": "check", "content": "x"}))).is_some());
        assert!(guard().judge("hermes", &hermes("patch", json!({"path": "tools/check/Cargo.toml", "old_string": "a", "new_string": "b"}))).is_some());
        assert!(guard().judge("hermes", &hermes("patch", json!({"mode": "patch", "patch": patch(".codex/hooks.json")}))).is_some());
        assert!(guard().judge("hermes", &hermes("terminal", json!({"command": "rm check", "workdir": "/p/my project"}))).is_some());
        assert!(guard().judge("hermes", &hermes("terminal", json!({"command": "rm -rf check/src", "workdir": "/p/my project/tools"}))).is_some());
        assert_eq!(guard().judge("hermes", &hermes("terminal", json!({"command": "rm -rf check/src", "workdir": "/p/my project/blocks"}))), None);
        assert_eq!(guard().judge("hermes", &hermes("write_file", json!({"path": "blocks/a/src/lib.rs", "content": "x"}))), None);
        let antigravity = |name: &str, args: Value| json!({"toolCall": {"name": name, "args": args}, "workspacePaths": ["/p/my project"]});
        assert!(guard().judge("antigravity", &antigravity("write_to_file", json!({"TargetFile": "/p/my project/check", "CodeContent": "x"}))).is_some());
        assert!(guard().judge("antigravity", &antigravity("replace_file_content", json!({"TargetFile": "/p/my project/tools/check/src/main.rs"}))).is_some());
        assert!(guard().judge("antigravity", &antigravity("run_command", json!({"CommandLine": "rm check", "Cwd": "/p/my project"}))).is_some());
        assert_eq!(guard().judge("antigravity", &antigravity("view_file", json!({"AbsolutePath": "/p/my project/blocks/a/CARD.md"}))), None);
        assert!(guard().judge("antigravity", &antigravity("view_file", json!({"AbsolutePath": "/p/my project/tools/check/src/main.rs"}))).is_some());
        assert_eq!(guard().judge("antigravity", &antigravity("list_dir", json!({"DirectoryPath": "/p/my project"}))), None);
        assert_eq!(guard().judge("antigravity", &antigravity("grep_search", json!({"Query": "check", "SearchPath": "/p/my project"}))), None);
        assert!(guard().judge("antigravity", &antigravity("mcp_write", json!({"target": {"path": "/p/my project/tools/check/x"}}))).is_some());
        assert!(guard().judge("codex", &json!({"tool_name": "mcp__fs__write", "tool_input": {"path": "/p/my project/.cargo/config.toml"}})).is_some());
        assert_eq!(guard().judge("codex", &json!({"tool_name": "mcp__fs__glob", "tool_input": {"path": "**/*.rs"}})), None);
        assert!(guard().judge("antigravity", &antigravity("write_to_file", json!({"Path": "/p/my project/check"}))).is_some());
        assert_eq!(guard().judge("antigravity", &antigravity("notify_user", json!({"Message": "tools/check/ 는 잠겨 있어서 rm check 를 못 합니다"}))), None);
        assert_eq!(guard().judge("hermes", &hermes("clarify", json!({"question": ".claude/settings.json 을 고쳐도 될까요? > check"}))), None);
        assert!(guard().judge("hermes", &hermes("execute_code", json!({"command": "rm -rf tools/check"}))).is_some());
        assert_eq!(guard().judge("codex", &json!({"tool_name": "mcp__x__create", "tool_input": {"description": "tools/check/src 는 건드리지 않음 (rm check 금지)"}})), None);
        assert_eq!(guard().judge("codex", &json!({"tool_name": "mcp__x__note", "tool_input": {"security": "check"}})), None);
        assert!(guard().judge("codex", &json!({"tool_name": "mcp__x__run", "tool_input": {"shell_command": "rm check"}})).is_some());
    }

    #[test]
    fn 뿌리_경로의_한글_정규화와_대소문자가_달라도_틀_파일을_알아본다() {
        let dir = std::env::temp_dir().join(format!("blockkit-guard-{}", std::process::id()));
        let root = dir.join("개인 데이터").join("Project");
        std::fs::create_dir_all(root.join("tools/check")).unwrap();
        let guard = Guard::new(&root, Path::new("/u"));
        let decomposed: String = "개인 데이터".chars().flat_map(|letter| {
            let code = letter as u32;
            if !(0xac00..=0xd7a3).contains(&code) {
                return vec![letter];
            }
            let index = code - 0xac00;
            let mut jamo = vec![char::from_u32(0x1100 + index / 588).unwrap(), char::from_u32(0x1161 + index % 588 / 28).unwrap()];
            if !index.is_multiple_of(28) {
                jamo.push(char::from_u32(0x11a7 + index % 28).unwrap());
            }
            jamo
        }).collect();
        assert_ne!(decomposed, "개인 데이터");
        for other in [dir.join(&decomposed).join("Project"), dir.join("개인 데이터").join("PROJECT")] {
            let path = other.join("tools/check/src/main.rs").to_string_lossy().into_owned();
            let input = json!({"toolCall": {"name": "write_to_file", "args": {"TargetFile": path}}, "workspacePaths": []});
            assert!(guard.judge("antigravity", &input).is_some(), "{path}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn 막는_답은_하네스마다_그_하네스의_모양이다() {
        let codex: Value = serde_json::from_str(&reply("codex", "check")).unwrap();
        assert_eq!(codex["hookSpecificOutput"]["permissionDecision"], "deny");
        assert!(codex["hookSpecificOutput"]["permissionDecisionReason"].as_str().unwrap().contains("check"));
        assert_eq!(serde_json::from_str::<Value>(&reply("antigravity", "check")).unwrap()["decision"], "deny");
        assert_eq!(serde_json::from_str::<Value>(&reply("hermes", "check")).unwrap()["decision"], "block");
        assert!(reply("hermes", "").contains("프로젝트 맨 위 폴더"));
    }
}
