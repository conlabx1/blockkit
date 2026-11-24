//! 블록 조립 규칙을 지켰는지 점검한다. 맨 위의 `check` 실행기가 이 프로그램을 빌드해서 부른다.
//!
//! 사용법: check --root <프로젝트 폴더> [--hook [codex|hermes|antigravity]] [--guard <codex|hermes|antigravity>] [--offline] [--limit <초>]
//!   --hook     답을 끝낼 때 도는 훅이 읽는 JSON 한 줄을 내고 항상 0 으로 끝난다(훅에서는 종료 코드 2 가 차단으로 읽힌다). 이름이 없으면 Claude Code 의 말로 낸다.
//!   --guard    도구를 쓰기 직전에 도는 훅으로 쓴다. 틀 파일을 고치는 호출이면 막는 JSON 한 줄을 내고, 아니면 아무것도 내지 않는다.
//!   --offline  인터넷을 쓰지 않는다. 의존성 관문은 「점검 못 함」으로 남는다.
//!   --limit    점검 전체에 쓸 시간(기본 240초). 넘기면 그 뒤에 도구를 불러야 하는 관문은 실패로 끝나고, 의존성 관문만 「점검 못 함」으로 남는다.
//!
//! 종료 코드: 0 전부 통과, 1 실패 있음(점검을 시작하지 못한 것도 실패다), 2 실패는 없지만 점검하지 못한 항목 있음.

mod gates;
mod guard;
mod picture;
mod run;
mod scan;

use gates::{FAIL, Gate, Project, SKIP};
use serde_json::{Value, json};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::OnceLock;
use std::thread;
use std::time::Duration;

/// Claude Code 가 훅에 넘겨 준 JSON. 다 읽기 전에는 비어 있다.
static HOOK_INPUT: OnceLock<String> = OnceLock::new();

/// 눈에 보이지 않으면서 글자의 순서나 줄을 바꾸는 글자의 범위(유니코드의 Cf, Zl, Zp 갈래).
const HIDDEN: [(char, char); 21] = [
    ('\u{ad}', '\u{ad}'),
    ('\u{600}', '\u{605}'),
    ('\u{61c}', '\u{61c}'),
    ('\u{6dd}', '\u{6dd}'),
    ('\u{70f}', '\u{70f}'),
    ('\u{890}', '\u{891}'),
    ('\u{8e2}', '\u{8e2}'),
    ('\u{180e}', '\u{180e}'),
    ('\u{200b}', '\u{200f}'),
    ('\u{2028}', '\u{202e}'),
    ('\u{2060}', '\u{2064}'),
    ('\u{2066}', '\u{206f}'),
    ('\u{feff}', '\u{feff}'),
    ('\u{fff9}', '\u{fffb}'),
    ('\u{110bd}', '\u{110bd}'),
    ('\u{110cd}', '\u{110cd}'),
    ('\u{13430}', '\u{1343f}'),
    ('\u{1bca0}', '\u{1bca3}'),
    ('\u{1d173}', '\u{1d17a}'),
    ('\u{e0001}', '\u{e0001}'),
    ('\u{e0020}', '\u{e007f}'),
];

/// 화면에 낼 글자에서 줄바꿈과 제어 문자를 물음표로 바꾼다. 파일 이름이나 카드 글자로 「결과:」 줄을 꾸며 낼 수 없게 한다.
fn clean(text: &str) -> String {
    text.chars().map(|letter| if letter.is_control() || HIDDEN.iter().any(|&(first, last)| (first..=last).contains(&letter)) { '?' } else { letter }).collect()
}

/// 한 줄을 결과에 더한다. 화면에 나가는 글자는 전부 여기를 거친다.
fn say(out: &mut String, line: &str) {
    out.push_str(&clean(line));
    out.push('\n');
}

/// 점검이 통째로 돌지 못한 상태를 「점검 못 함」으로 두면 그사이의 실패가 가려진다. 그래서 실패로 본다.
fn not_started(reason: &str) -> (u8, String) {
    (1, format!("{FAIL} 점검: 시작하지 못했습니다. {}\n\n결과: 실패 1개(점검을 시작하지 못했습니다)\n", clean(reason)))
}

fn stopped(root: &Path, reason: &str) -> (u8, String) {
    let stopped = not_started(reason);
    picture::save(root, &[], &[("점검", Gate { status: FAIL, summary: "시작하지 못했습니다".to_string(), details: vec![clean(reason)] })], stopped.0, &stopped.1);
    stopped
}

/// 지금 무엇으로 조립돼 있는지 보여 준다. 숫자가 늘었으면 왜 늘었는지 물어볼 수 있게 한다.
fn describe(project: &Project, out: &mut String) {
    for (kind, label) in [("blocks", "블록"), ("doors", "통로")] {
        let mine: Vec<_> = project.units.iter().filter(|unit| unit.kind == kind).collect();
        say(out, &format!("{label} {}개", mine.len()));
        for unit in mine {
            let card = project.card(unit);
            let purpose = card.get("하는 일").and_then(|lines| lines.first()).map_or("(카드에 설명이 없습니다)", String::as_str);
            say(out, &format!("  {}: {purpose}", unit.name));
        }
    }
    let externals = project.externals();
    say(out, &format!("외부 부품 {}개{}{}", externals.len(), if externals.is_empty() { "" } else { ": " }, externals.join(", ")));
    let waived: usize = project.exceptions.values().map(Vec::len).sum();
    say(out, &format!("예외 {}개(블록과 조립에 허용한 외부 부품 {}개, 끈 패턴 검사 {waived}개)", project.extra.len() + waived, project.extra.len()));
    say(out, "");
}

/// 점검을 돌려서 (종료 코드, 화면에 낼 결과) 를 돌려준다.
fn check(root: &Path, offline: bool, limit: Duration) -> (u8, String) {
    let project = match Project::load(root, offline, limit) {
        Ok(project) => project,
        Err(reason) => return stopped(root, &reason),
    };
    let mut out = String::new();
    describe(&project, &mut out);
    // 「금지 연결」 관문은 「시험」 관문에서 실제로 돌아간 시험의 목록을 받아서 쓴다.
    let (tests, executed) = gates::tests(&project);
    let results = [
        ("카드", gates::cards(&project)),
        ("시험", tests),
        ("금지 연결", gates::links(&project, &executed)),
        ("통로", gates::doors(&project)),
        ("패턴", gates::patterns(&project)),
        ("비밀 값", gates::secrets(&project)),
        ("장치", gates::devices(&project)),
        ("의존성", gates::dependencies(&project)),
    ];
    for (name, gate) in &results {
        say(&mut out, &format!("{} {name}: {}", gate.status, gate.summary));
        for line in &gate.details {
            say(&mut out, &format!("     {line}"));
        }
    }
    say(&mut out, "");
    let count = |status: &str| results.iter().filter(|(_, gate)| gate.status == status).count();
    let (failed, skipped) = (count(FAIL), count(SKIP));
    let finished = if failed > 0 {
        say(&mut out, &format!("결과: 실패 {failed}개"));
        say(&mut out, "  에이전트에게 이 결과를 보여 주고 「점검 실패를 고쳐 줘」라고 말하면 됩니다.");
        (1, out)
    } else if skipped > 0 {
        say(&mut out, &format!("결과: 실패는 없지만 점검하지 못한 항목이 {skipped}개 있습니다."));
        say(&mut out, "  인터넷 연결을 확인하고 다시 돌립니다. 그래도 같으면 에이전트에게 이 결과를 보여 줍니다.");
        (2, out)
    } else {
        say(&mut out, "결과: 전부 통과");
        (0, out)
    };
    picture::save(root, &picture::bricks(&project), &results, finished.0, &finished.1);
    finished
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Harness {
    Claude,
    Codex,
    Hermes,
    Antigravity,
}

fn harness_of(args: &[String]) -> Option<Harness> {
    let at = args.iter().position(|arg| arg == "--hook")?;
    Some(match args.get(at + 1).map(String::as_str) {
        Some("codex") => Harness::Codex,
        Some("hermes") => Harness::Hermes,
        Some("antigravity") => Harness::Antigravity,
        _ => Harness::Claude,
    })
}

/// 답을 끝낼 때 도는 훅의 답을 만든다. `input` 은 하네스가 훅에 넘겨 준 JSON 이다.
fn hook_reply(harness: Harness, input: &str, output: &str) -> String {
    let data = serde_json::from_str::<Value>(input).unwrap_or(Value::Null);
    // 줄 머리의 「결과:」만 본다. 카드 설명 같은 다른 줄에 섞인 글자를 결과로 읽지 않는다.
    let verdict = output.lines().find(|line| line.starts_with("결과:")).unwrap_or(output.trim());
    let summary = format!("점검(./check) {verdict}");
    // 의존성 실패는 방금 고친 것과 무관하게 새 취약점 공개로 생길 수 있어서, 알리기만 하고 막지는 않는다.
    let blocking = output.lines().any(|line| line.starts_with(FAIL) && !line.starts_with(&format!("{FAIL} 의존성")));
    let reason = format!("점검(./check)에 실패가 있습니다. 고치고 다시 점검하거나, 고치지 못하면 실패 상태임을 보고에 그대로 적으십시오.\n\n{output}");
    let active = match harness {
        Harness::Hermes => data["extra"]["attempt"].as_u64().is_some_and(|attempt| attempt > 0),
        Harness::Antigravity => data["terminationReason"] != "NO_TOOL_CALL" || data["fullyIdle"] != true || data["continuationActive"] == true,
        _ => data["stop_hook_active"] == true,
    };
    if harness == Harness::Antigravity {
        let reply = if blocking && !active { json!({"decision": "continue", "reason": reason}) } else { json!({"decision": "stop", "reason": summary}) };
        return reply.to_string();
    }
    // 매번 한 줄을 화면에 보여 준다. 이 줄이 안 보이면 훅이 돌지 않은 것이다.
    let mut reply = json!({"systemMessage": summary});
    // 이미 한 번 막아서 이어 가는 중이면 다시 막지 않는다(끝없이 도는 것을 막는다). 실패 상태는 위 한 줄로 보인다.
    if blocking && !active {
        reply["decision"] = json!("block");
        reply["reason"] = json!(reason);
    }
    reply.to_string()
}

/// 결과를 화면에 내고 종료 코드를 돌려준다. 훅이면 JSON 한 줄을 내고 0 을 돌려준다.
fn report(hook: Option<Harness>, code: u8, output: &str) -> u8 {
    let Some(harness) = hook else {
        print!("{output}");
        return code;
    };
    println!("{}", hook_reply(harness, HOOK_INPUT.get().map_or("", String::as_str), output));
    0
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| args.iter().any(|arg| arg == name);
    let value = |name: &str| args.iter().position(|arg| arg == name).and_then(|at| args.get(at + 1));
    let hook = harness_of(&args);
    if let Some(at) = args.iter().position(|arg| arg == "--guard") {
        let mut input = String::new();
        let _ = std::io::stdin().read_to_string(&mut input);
        let data = serde_json::from_str(&input).unwrap_or(Value::Null);
        let harness = args.get(at + 1).map_or("", String::as_str);
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        if let Some(path) = value("--root").and_then(|root| guard::Guard::new(Path::new(root), &home).judge(harness, &data)) {
            println!("{}", guard::reply(harness, &path));
        }
        return ExitCode::SUCCESS;
    }
    // 훅의 시간 한도(.claude/settings.json 의 300초)보다 먼저 스스로 끝나야 결과를 알릴 수 있다.
    let limit = value("--limit").and_then(|seconds| seconds.parse().ok()).unwrap_or(240);
    // 점검이 어디선가 멈춰도 실패로 끝나게 한다. 훅이 시간 한도에 걸려 끊기면 막지도 알리지도 못한 채 지나간다.
    let root = value("--root").map(PathBuf::from);
    thread::spawn(move || {
        thread::sleep(Duration::from_secs(limit + 5 + limit / 16));
        let reason = "시간 안에 끝나지 않았습니다.";
        let (code, output) = root.map_or_else(|| not_started(reason), |root| stopped(&root, reason));
        std::process::exit(report(hook, code, &output).into());
    });
    if hook.is_some() {
        let mut input = String::new();
        let _ = std::io::stdin().read_to_string(&mut input);
        let _ = HOOK_INPUT.set(input);
    }
    let (code, output) = match value("--root") {
        Some(root) => check(Path::new(root), flag("--offline"), Duration::from_secs(limit)),
        None => not_started("점검할 폴더(--root)를 받지 못했습니다."),
    };
    ExitCode::from(report(hook, code, &output))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 줄을_바꾸거나_숨는_글자는_물음표로_바꾼다() {
        assert_eq!(clean("가\n나\r다\u{1b}[2K라\u{2028}마\u{202e}바\u{200b}사"), "가?나?다?[2K라?마?바?사");
        assert_eq!(clean("✅ ⚠️ 한글 abc ―「」"), "✅ ⚠️ 한글 abc ―「」");
    }

    const FAILED: &str = "❌ 카드: 1건\n\n결과: 실패 1개\n";

    fn reply(harness: Harness, input: &str, output: &str) -> Value {
        serde_json::from_str(&hook_reply(harness, input, output)).unwrap()
    }

    #[test]
    fn 훅은_실패일_때만_막고_의존성_실패와_이미_막은_경우는_알리기만_한다() {
        for harness in [Harness::Claude, Harness::Codex] {
            let answer = reply(harness, "{}", FAILED);
            assert_eq!((answer["systemMessage"].as_str(), answer["decision"].as_str()), (Some("점검(./check) 결과: 실패 1개"), Some("block")));
            assert!(answer["reason"].as_str().unwrap().contains("❌ 카드"));
            for (input, output) in [("{\"stop_hook_active\": true}", FAILED), ("{}", "❌ 의존성: 취약점\n\n결과: 실패 1개\n"), ("{}", "✅ 카드: 좋음\n\n결과: 전부 통과\n")] {
                let answer = reply(harness, input, output);
                assert_eq!(answer.as_object().map(|fields| fields.len()), Some(1), "{answer}");
            }
            // 입력이 이상해도 실패는 막는다.
            for input in ["[]", "null", "", "{\"stop_hook_active\": tr"] {
                assert_eq!(reply(harness, input, FAILED)["decision"], "block", "{input}");
            }
        }
    }

    #[test]
    fn 헤르메스는_이번_턴에_처음_멈출_때만_막는다() {
        let input = |attempt: &str| format!("{{\"hook_event_name\": \"pre_verify\", \"session_id\": \"s\", \"extra\": {{\"attempt\": {attempt}, \"changed_paths\": [\"a.rs\"]}}}}");
        let first = reply(Harness::Hermes, &input("0"), FAILED);
        assert_eq!(first["decision"], "block");
        assert!(first["reason"].as_str().unwrap().contains("❌ 카드"));
        let again = reply(Harness::Hermes, &input("1"), FAILED);
        assert!(again.get("decision").is_none() && again["systemMessage"] == "점검(./check) 결과: 실패 1개", "{again}");
        assert_eq!(reply(Harness::Hermes, "{}", FAILED)["decision"], "block");
        assert!(reply(Harness::Hermes, &input("0"), "✅ 카드: 좋음\n\n결과: 전부 통과\n").get("decision").is_none());
    }

    #[test]
    fn 안티그래비티는_스스로_멈춘_턴에서_한_번만_되돌린다() {
        let input = |reason: &str, idle: bool, active: Option<bool>| {
            let mut data = json!({"conversationId": "c-1", "lastUserInput": "블록 하나 만들어 줘", "executionNum": 0, "terminationReason": reason, "fullyIdle": idle});
            if let Some(active) = active {
                data["continuationActive"] = json!(active);
            }
            data.to_string()
        };
        for active in [Some(false), None] {
            let first = reply(Harness::Antigravity, &input("NO_TOOL_CALL", true, active), FAILED);
            assert_eq!(first["decision"], "continue", "{active:?}");
            assert!(first["reason"].as_str().unwrap().contains("❌ 카드"));
        }
        let stopped = json!({"decision": "stop", "reason": "점검(./check) 결과: 실패 1개"});
        for data in [input("NO_TOOL_CALL", true, Some(true)), input("NO_TOOL_CALL", false, Some(false)), input("model_stop", true, Some(false)), input("MAX_INVOCATIONS", true, None), input("ERROR", true, None), "{}".to_string(), String::new()] {
            assert_eq!(reply(Harness::Antigravity, &data, FAILED), stopped, "{data}");
        }
        for output in ["✅ 카드: 좋음\n\n결과: 전부 통과\n", "❌ 의존성: 취약점\n\n결과: 실패 1개\n"] {
            assert_eq!(reply(Harness::Antigravity, &input("NO_TOOL_CALL", true, Some(false)), output)["decision"], "stop");
        }
    }

    #[test]
    fn 훅_방언은_훅_바로_뒤의_이름으로_정한다() {
        let args = |list: &[&str]| list.iter().map(|arg| arg.to_string()).collect::<Vec<_>>();
        assert_eq!(harness_of(&args(&["--root", "/p", "--offline"])), None);
        assert_eq!(harness_of(&args(&["--root", "/p", "--hook"])), Some(Harness::Claude));
        assert_eq!(harness_of(&args(&["--hook", "--offline"])), Some(Harness::Claude));
        for (name, harness) in [("codex", Harness::Codex), ("hermes", Harness::Hermes), ("antigravity", Harness::Antigravity)] {
            assert_eq!(harness_of(&args(&["--root", "/p", "--hook", name, "--offline"])), Some(harness));
        }
        assert_eq!(harness_of(&args(&["--hook", "gemini"])), Some(Harness::Claude));
    }
}
