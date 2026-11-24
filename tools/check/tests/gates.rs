//! 점검 명령 자체를 검증한다. 임시 폴더에 작은 프로젝트를 만들고 위반을 하나씩 심어서 실패가 나오는지 본다.
//!
//! 실행: cargo test --locked --manifest-path tools/check/Cargo.toml
//! 점검 명령(tools/check)이나 실행기(check)를 고쳤을 때 돌린다. ./check 의 「시험」 관문은 이 파일을 돌리지 않는다.
//! 이 시험은 프로젝트의 블록에 기대지 않는다. 예시 블록을 지우거나 바꿔도 그대로 돈다.

use serde_json::{Value, json};
use std::fs;
use std::io::Write;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

const BLOCK: &str = "blocks/sample/src/lib.rs";
const CARD: &str = "blocks/sample/CARD.md";
const APP: &str = "app/src/main.rs";
const DEVICES: &str = ".claude/settings.json";
/// 프로젝트가 예외를 더해도 이 시험의 뜻이 바뀌지 않게, 임시 프로젝트의 점검 설정은 여기서 고정한다.
const ROOT_MANIFEST: &str = r#"[workspace]
resolver = "3"
members = ["app"]

[workspace.metadata.blockkit]
door-crates = []
extra-crates = []

[workspace.metadata.blockkit.pattern-exceptions]
"#;
const SAMPLE: [(&str, &str); 7] = [
    ("Cargo.toml", ROOT_MANIFEST),
    ("blocks/sample/Cargo.toml", "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = false\n"),
    (
        "app/Cargo.toml",
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = false\n\n[dependencies]\nsample = { path = \"../blocks/sample\" }\n",
    ),
    (
        APP,
        r#"fn main() -> Result<(), String> {
    let text = std::env::args().skip(1).collect::<Vec<_>>().join(" ");
    println!("{}", sample::shout(&text)?);
    Ok(())
}
"#,
    ),
    (
        "app/tests/run.rs",
        r#"use std::process::Command;

#[test]
fn 전체가_돈다() {
    let done = Command::new(env!("CARGO_BIN_EXE_app")).arg("hi").output().unwrap();
    assert_eq!(String::from_utf8_lossy(&done.stdout), "HI\n");
}
"#,
    ),
    (
        CARD,
        "# sample\n\n## 하는 일\n글자를 대문자로 바꾼다.\n\n## 입력\n- 글자\n\n## 출력\n- 대문자 글자\n\n## 금지 사항\n- [N1] 빈 글자는 받지 않는다.\n- [N2] 줄바꿈이 든 글자는 받지 않는다.\n",
    ),
    (
        BLOCK,
        r#"pub fn shout(text: &str) -> Result<String, String> {
    if text.is_empty() {
        return Err("빈 글자".to_string());
    }
    if text.contains('\n') {
        return Err("줄바꿈".to_string());
    }
    Ok(text.to_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 대문자로_바꾼다() {
        assert_eq!(shout("hi"), Ok("HI".to_string()));
    }

    #[test]
    fn n1_빈_글자는_받지_않는다() {
        assert!(shout("").is_err());
    }

    #[test]
    fn n2_줄바꿈이_든_글자는_받지_않는다() {
        assert!(shout("a\nb").is_err());
    }
}
"#,
    ),
];
const UNIT_CARD: &str = "# 이름\n\n## 하는 일\n값을 하나 돌려준다.\n\n## 입력\n- 없음\n\n## 출력\n- 글자\n\n## 금지 사항\n- [N1] 빈 값을 돌려주지 않는다.\n";
const UNIT_TEST: &str = "\n#[cfg(test)]\nmod tests {\n    use super::*;\n\n    #[test]\n    fn n1_빈_값을_돌려주지_않는다() {\n        assert!(!run().is_empty());\n    }\n}\n";
const PLAIN_CODE: &str = "pub fn run() -> String {\n    \"ok\".to_string()\n}\n";
const DOOR_CODE: &str = "pub fn run() -> String {\n    std::env::var(\"HOME\").unwrap_or_else(|_| \"home\".to_string())\n}\n";

/// 이 파일 자체가 비밀 값 검사에 걸리지 않게 나눠 적는다.
fn secret() -> String {
    ["AKIA", "ABCDEFGHIJKLMNOP"].concat()
}

fn with_test(code: &str) -> String {
    format!("{code}{UNIT_TEST}")
}

/// 틀의 맨 위 폴더에 있는 실제 파일
fn real(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(name)
}

/// 이 시험을 돌리고 있는 cargo 와 같은 자리의 도구(cargo, rustc, clippy)가 먼저 잡히는 PATH
fn path() -> String {
    let cargo = std::env::var("CARGO").unwrap();
    format!("{}:{}", Path::new(&cargo).parent().unwrap().display(), std::env::var("PATH").unwrap())
}

fn temp_dir() -> PathBuf {
    static COUNT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!("blockkit-{}-{}", std::process::id(), COUNT.fetch_add(1, Ordering::Relaxed)));
    let _ = fs::remove_dir_all(&dir);
    dir
}

fn script(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// 프로그램을 돌려서 (종료 코드, 화면에 낸 글자) 를 돌려준다.
fn output_of(mut command: Command, stdin: &str) -> (i32, String) {
    let mut child = command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
    let done = child.wait_with_output().unwrap();
    (done.status.code().unwrap_or(-1), String::from_utf8_lossy(&done.stdout).into_owned())
}

/// 결과에서 실패한 관문 하나의 요약 줄과 그 아래의 설명 줄만 떼어 낸다.
fn section(output: &str, gate: &str) -> String {
    let mut lines = output.lines().skip_while(|line| !line.starts_with(&format!("❌ {gate}:")));
    let first = lines.next().unwrap_or_default();
    let rest: Vec<&str> = lines.take_while(|line| line.starts_with("     ")).collect();
    format!("{first}\n{}", rest.join("\n"))
}

/// 규칙을 지킨 작은 프로젝트. 시험이 끝나면 지워진다.
struct Project {
    dir: PathBuf,
    root: PathBuf,
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

impl Project {
    fn new() -> Project {
        let dir = temp_dir();
        // 실제 프로젝트처럼 경로에 빈칸이 있어도 돌아야 한다.
        let project = Project { root: dir.join("my project"), dir };
        for (name, text) in SAMPLE {
            project.write(name, text);
        }
        // 실제 설정 파일을 그대로 쓴다. 틀의 설정 파일이 점검 명령이 정한 내용과 어긋나면 이 시험들이 실패한다.
        for name in [DEVICES, "rust-toolchain.toml", ".codex/hooks.json", ".agents/hooks.json"] {
            project.write(name, &fs::read_to_string(real(name)).unwrap());
        }
        assert!(project.lock(true));
        project
    }

    fn write(&self, name: &str, text: &str) {
        let path = self.root.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn read(&self, name: &str) -> String {
        fs::read_to_string(self.root.join(name)).unwrap()
    }

    fn edit(&self, name: &str, old: &str, new: &str) {
        let text = self.read(name);
        assert!(text.contains(old), "{name} 에 바꿀 자리가 없다: {old}"); // 바꿀 자리가 없으면 위반이 심기지 않은 것이다.
        self.write(name, &text.replace(old, new));
    }

    fn append(&self, name: &str, text: &str) {
        self.write(name, &(self.read(name) + text));
    }

    fn remove(&self, name: &str) {
        fs::remove_file(self.root.join(name)).unwrap();
    }

    /// Cargo.lock 을 Cargo.toml 에 맞춘다. 맞추지 못하면 false 를 돌려준다.
    fn lock(&self, offline: bool) -> bool {
        let mut command = Command::new(std::env::var("CARGO").unwrap());
        command.arg("generate-lockfile").args(offline.then_some("--offline")).current_dir(&self.root).env("PATH", path());
        command.stdout(Stdio::null()).stderr(Stdio::null()).status().unwrap().success()
    }

    /// 블록이나 통로를 하나 더하고 조립에 잇는다.
    fn add_unit(&self, kind: &str, name: &str, code: &str) {
        self.write(&format!("{kind}/{name}/CARD.md"), UNIT_CARD);
        self.write(&format!("{kind}/{name}/Cargo.toml"), &format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = false\n"));
        self.write(&format!("{kind}/{name}/src/lib.rs"), code);
        self.append("app/Cargo.toml", &format!("{name} = {{ path = \"../{kind}/{name}\" }}\n"));
        assert!(self.lock(true));
    }

    fn check_with(&self, args: &[&str], stdin: &str, env: &[(&str, &str)]) -> (i32, String) {
        let mut command = Command::new(env!("CARGO_BIN_EXE_check"));
        command.arg("--root").arg(&self.root).args(args).env("PATH", path()).envs(env.iter().copied());
        output_of(command, stdin)
    }

    /// 인터넷은 의존성 관문만 쓴다. 그 관문을 보는 시험만 인터넷을 켠다.
    fn check(&self) -> (i32, String) {
        self.check_with(&["--offline"], "", &[])
    }

    fn passes(&self) -> String {
        let (code, output) = self.check();
        assert!(!output.contains('❌'), "{output}");
        assert_eq!(code, 2, "{output}"); // 인터넷을 꺼 두었으므로 의존성만 「점검 못 함」으로 남는다.
        output
    }

    fn fails(&self, gate: &str, expected: &[&str]) -> String {
        let (code, output) = self.check();
        assert_eq!(code, 1, "{output}");
        for text in [format!("❌ {gate}")].iter().map(String::as_str).chain(expected.iter().copied()) {
            assert!(output.contains(text), "「{text}」 가 없다:\n{output}");
        }
        output
    }

    fn hook(&self, stdin: &str) -> Value {
        let (code, output) = self.check_with(&["--hook", "--offline"], stdin, &[]);
        assert_eq!(code, 0, "{output}"); // 훅은 무슨 일이 있어도 0 으로 끝나야 한다.
        serde_json::from_str(&output).unwrap()
    }
}

// ---- 기준: 규칙을 지킨 프로젝트는 실패가 없다 ----

#[test]
fn 깨끗한_프로젝트는_실패가_없다() {
    let project = Project::new();
    let output = project.passes();
    for gate in ["카드", "시험", "금지 연결", "통로", "패턴", "비밀 값", "장치"] {
        assert!(output.contains(&format!("✅ {gate}")), "{output}");
    }
    assert!(output.contains("✅ 시험: 4개 통과"), "{output}");
    assert!(output.contains("⚠️ 의존성") && output.contains("인터넷"), "{output}");
    assert!(output.starts_with("블록 1개\n  sample: 글자를 대문자로 바꾼다.\n통로 0개\n외부 부품 0개\n예외 0개("), "{output}");
}

#[test]
fn 점검이_돌_때마다_그림_페이지가_지금_결과로_바뀐다() {
    let project = Project::new();
    project.passes();
    let page = project.read("target/check.html");
    assert!(page.contains(r#"class="verdict skip""#), "{page}");
    assert!(page.contains(r#"class="brick name ok" data-unit="blocks/sample""#), "{page}");
    assert!(page.contains(r#"class="brick ok" data-id="N1""#), "{page}");
    project.edit(BLOCK, "fn n1_빈_글자는_받지_않는다", "fn 빈_글자는_받지_않는다");
    project.fails("금지 연결", &["blocks/sample: 금지 사항 [N1] 을 확인하는 시험이 없습니다."]);
    let page = project.read("target/check.html");
    assert!(page.contains(r#"class="verdict fail""#), "{page}");
    assert!(page.contains(r#"class="brick name fail" data-unit="blocks/sample""#), "{page}");
    assert!(page.contains(r#"class="brick fail" data-id="N1""#), "{page}");
    assert!(page.contains(r#"class="brick ok" data-id="N2""#), "{page}");
    project.remove("target/check.html");
    assert_eq!(project.hook("{}")["decision"], "block");
    assert!(project.read("target/check.html").contains(r#"class="brick fail" data-id="N1""#));
}

#[test]
fn 블록과_통로를_더해도_실패가_없다() {
    let project = Project::new();
    project.add_unit("blocks", "second", &with_test(PLAIN_CODE));
    project.add_unit("doors", "home", &with_test(DOOR_CODE));
    let output = project.passes();
    for text in ["블록 2개", "통로 1개", "6개 통과"] {
        assert!(output.contains(text), "{output}");
    }
}

#[test]
fn 규칙을_지킨_흔한_모양은_실패로_보지_않는다() {
    let project = Project::new();
    // 블록 안의 다른 파일, 허용된 표준 라이브러리, 이름 하나만 허용된 것, 범위(..) 뒤에 온 허용된 경로
    let rules = r#"use std::collections::BTreeMap;
use std::sync::{Arc, LazyLock, OnceLock};
use std::time::Duration;

static WORDS: LazyLock<Arc<str>> = LazyLock::new(|| Arc::from("a"));
static ONCE: OnceLock<usize> = OnceLock::new();

pub fn clean(text: &str) -> String {
    let _table: BTreeMap<String, Duration> = BTreeMap::new();
    let _most = (0..std::cmp::max(WORDS.len(), *ONCE.get_or_init(|| 1))).count();
    text.trim().to_string()
}
"#;
    project.write("blocks/sample/src/rules.rs", rules);
    project.edit(BLOCK, "pub fn shout", "mod rules;\n\npub fn shout");
    project.edit(BLOCK, "Ok(text.to_uppercase())", "Ok(rules::clean(text).to_uppercase())");
    // 조립 파일에서 약속을 구현하는 `impl … for …` 는 판단이 아니다.
    project.edit(APP, "sample::shout(&text)?);", "Shown(sample::shout(&text)?));");
    project.append(APP, "\nstruct Shown(String);\n\nimpl std::fmt::Display for Shown {\n    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {\n        out.write_str(&self.0)\n    }\n}\n");
    // 통로는 바깥을 쓴다. tests/ 폴더의 시험은 블록의 것이라도 바깥을 쓸 수 있다.
    project.add_unit("doors", "wait", &with_test(DOOR_CODE));
    // 줄바꿈 없이 화면에 깨진 바이트와 결과 줄처럼 생긴 글자를 남기고, 다른 폴더로 옮겨 가서 일하는 시험
    let noisy = "use std::io::Write;\n\n#[test]\nfn 화면에_글자를_남기고_폴더를_옮겨도_된다() {\n    std::io::stdout().write_all(b\"\\xff test result: FAILED\").unwrap();\n    std::env::set_current_dir(std::env::temp_dir()).unwrap();\n    assert_eq!(sample::shout(\"a\"), Ok(\"A\".to_string()));\n}\n";
    project.write("blocks/sample/tests/outside.rs", noisy);
    // 글자 파일이 아닌 파일은 비밀 값 검사에서 뺀다.
    fs::write(project.root.join("blocks/sample/table.bin"), [b"\0".as_slice(), secret().as_bytes()].concat()).unwrap();
    // 맨 위의 숨김 폴더에 든 코드는 자리 검사에서 빼고, 점검 명령의 코드는 정해진 자리로 본다.
    project.write(".claude/hooks/helper.rs", "fn main() {}\n");
    project.write("tools/check/src/main.rs", "fn main() {}\n");
    // Claude Code 가 .claude/worktrees/ 에 만드는 작업 사본, 맨 위의 저장소 폴더와 빌드 폴더는 보지 않는다.
    for name in [".claude/worktrees/fix/stray.rs", "target/stray.rs", "tools/check/target/stray.rs"] {
        project.write(name, "fn main() {}\n");
    }
    for name in [".claude/worktrees/fix/notes.txt", ".git/key.txt", "target/key.txt", "tools/check/target/key.txt"] {
        project.write(name, &secret());
    }
    assert!(project.passes().contains("6개 통과"));
}

// ---- 카드 ----

#[test]
fn 카드가_없거나_항목이_빠지면_실패한다() {
    let project = Project::new();
    project.edit(CARD, "## 금지 사항", "## 메모");
    project.edit(CARD, "- 대문자 글자\n", ""); // 제목만 있고 내용이 없는 항목
    project.fails("카드", &["blocks/sample: 카드에 「## 금지 사항」 항목이 없거나 비어 있습니다.", "blocks/sample: 카드에 「## 출력」 항목이 없거나 비어 있습니다."]);
    project.remove(CARD);
    project.fails("카드", &["blocks/sample: CARD.md 가 없습니다."]);
}

#[test]
fn 번호가_없거나_겹친_금지_사항은_실패한다() {
    let project = Project::new();
    project.edit(CARD, "- [N2] 줄바꿈", "- 남의 글자는 받지 않는다.\n- [N] 번호가 비었다.\n- [N3a] 번호에 글자가 섞였다.\n* [N4] 별표로 시작했다.\n- [N1] 욕설은 받지 않는다.\n- [N2] 줄바꿈");
    let expected = [
        "「- [N1] 내용」 형태로 적습니다: 「- 남의 글자는",
        "형태로 적습니다: 「- [N] 번호가 비었다.",
        "형태로 적습니다: 「- [N3a] 번호에",
        "형태로 적습니다: 「* [N4] 별표로",
        "금지 사항 번호 [N1] 이 겹칩니다.",
    ];
    let output = project.fails("카드", &expected);
    assert!(!output.contains("[N2] 이 겹칩니다"), "{output}");
}

#[test]
fn 금지_사항_제목이_두_번_나와도_앞의_약속이_사라지지_않는다() {
    let project = Project::new();
    let old = "- [N2] 줄바꿈이 든 글자는 받지 않는다.\n";
    project.edit(CARD, old, &format!("{old}\n## 금지 사항\n- [N1] 빈 글자는 받지 않는다.\n"));
    project.fails("카드", &["금지 사항 번호 [N1] 이 겹칩니다."]);
}

#[test]
fn 정해진_자리_밖의_코드는_실패한다() {
    let project = Project::new();
    let names = ["stray.rs", "app/src/extra.rs", "blocks/loose.rs", "blocks/sample/build.rs", "blocks/sample/examples/demo.rs", "tools/other.rs"];
    for name in names {
        project.write(name, "pub const VALUE: u8 = 1;\n");
    }
    project.fails("카드", &names.map(|name| format!("{name}: 정해진 자리 밖에 놓인 코드 파일입니다.")).each_ref().map(String::as_str));
}

#[test]
fn 블록의_뼈대가_빠지면_실패한다() {
    let project = Project::new();
    project.add_unit("blocks", "second", &with_test(PLAIN_CODE));
    // 조립에 이어지지 않은 블록은 빌드도 시험도 되지 않는다.
    project.edit("app/Cargo.toml", "second = { path = \"../blocks/second\" }\n", "");
    assert!(project.lock(true));
    project.write("blocks/Bad-Name/CARD.md", UNIT_CARD);
    let expected = [
        "blocks/second: app/Cargo.toml 의 [dependencies] 에 이어져 있지 않습니다.",
        "blocks/Bad-Name: 폴더 이름은 영어 소문자로 시작하고",
        "blocks/Bad-Name: 설정 파일(Cargo.toml)이 없습니다.",
        "blocks/Bad-Name: 코드 파일(src/lib.rs)이 없습니다.",
    ];
    let output = project.fails("카드", &expected);
    assert!(output.contains("blocks/second: 실행된 시험이 없습니다."), "{output}");
    // 시험에만 이은 블록은 시험은 돌지만 조립된 프로그램에는 들어가지 않는다.
    project.append("app/Cargo.toml", "\n[dev-dependencies]\nsecond = { path = \"../blocks/second\" }\n");
    assert!(project.lock(true));
    let output = project.fails("카드", &expected[..1]);
    assert!(!output.contains("blocks/second: 실행된 시험이 없습니다."), "{output}");
}

#[test]
fn 조립_시험이_없으면_실패한다() {
    let project = Project::new();
    project.remove("app/tests/run.rs");
    let output = project.fails("카드", &["app/tests: 조립된 전체를 실제로 돌려 보는 시험이 없습니다."]);
    assert!(output.contains("app: 실행된 시험이 없습니다."), "{output}");
}

#[test]
fn 카드를_읽지_못해도_통과로_보지_않는다() {
    let project = Project::new();
    // 다른 인코딩(EUC-KR)으로 저장된 카드: 「## 하는 일」
    fs::write(project.root.join(CARD), b"# sample\n\n## \xc7\xcf\xb4\xc2 \xc0\xcf\nx\n").unwrap();
    project.fails("카드", &["카드에 「## 하는 일」 항목이 없거나 비어 있습니다."]);
}

// ---- 시험 ----

#[test]
fn 블록이_약속을_어기면_깨진_약속의_이름이_나온다() {
    let project = Project::new();
    project.edit(BLOCK, "if text.is_empty() {", "if text == \"없음\" {");
    project.edit(BLOCK, "Ok(text.to_uppercase())", "Ok(text.to_lowercase())");
    project.fails("시험", &["4개 가운데 실패 3개", "실패: blocks/sample: n1_빈_글자는_받지_않는다", "실패: 조립(app): 전체가_돈다"]);
}

#[test]
fn 건너뛴_시험은_실패한다() {
    let project = Project::new();
    project.edit(BLOCK, "    #[test]\n    fn n1_", "    #[test]\n    #[ignore]\n    fn n1_");
    let output = project.fails("시험", &["건너뜀: blocks/sample: n1_빈_글자는_받지_않는다"]);
    assert!(output.contains("❌ 금지 연결") && output.contains("금지 사항 [N1] 의 시험이 실제로 실행되지 않았습니다."), "{output}");
}

#[test]
fn 시험이_없는_블록은_실패한다() {
    let project = Project::new();
    project.add_unit("blocks", "second", PLAIN_CODE);
    project.fails("시험", &["blocks/second: 실행된 시험이 없습니다.", "시험이 없는 곳 1개"]);
}

#[test]
fn 빌드되지_않으면_실패하고_자리를_알려_준다() {
    let project = Project::new();
    project.edit(BLOCK, "Ok(text.to_uppercase())", "Ok(text.len())");
    let output = project.fails("시험", &["시험을 빌드하지 못했습니다"]);
    // 같은 오류가 「패턴」 줄에도 나오므로, 「시험」 줄 아래에 자리가 적혔는지 따로 본다.
    assert!(section(&output, "시험").contains("blocks/sample/src/lib.rs:8: mismatched types [E0308]"), "{output}");
}

#[test]
fn 이어지지_않은_파일의_시험은_돌지_않은_것으로_나온다() {
    let project = Project::new();
    project.write("blocks/sample/src/more.rs", "#[test]\nfn 따로_둔_시험() {\n    assert_eq!(1 + 1, 2);\n}\n");
    project.fails("시험", &["돌지 않음: blocks/sample: 따로_둔_시험 (blocks/sample/src/more.rs)", "돌지 않은 시험 1개"]);
}

#[test]
fn 이어지지_않은_파일의_시험은_도는_시험과_이름이_같아도_돌지_않은_것으로_나온다() {
    let project = Project::new();
    project.write("blocks/sample/src/more.rs", "#[test]\nfn 대문자로_바꾼다() {\n    assert_eq!(1 + 1, 2);\n}\n");
    project.fails("시험", &["돌지 않음: blocks/sample: 대문자로_바꾼다 (", "blocks/sample/src/more.rs", "돌지 않은 시험 1개"]);
}

#[test]
fn 시험_도중에_프로그램을_끝내면_통과로_치지_않는다() {
    let project = Project::new();
    project.write("blocks/sample/tests/quit.rs", "#[test]\nfn 조용히_끝낸다() {\n    std::process::exit(0);\n}\n");
    // 통과를 알리는 줄처럼 생긴 글자를 찍고 실패하는 시험
    project.write("blocks/sample/tests/fake.rs", "#[test]\nfn 통과한_척한다() {\n    println!(\"\\ntest result: ok. 1 passed; 0 failed\");\n    panic!(\"실패\");\n}\n");
    project.fails("시험", &["실패: blocks/sample: 조용히_끝낸다", "실패: blocks/sample: 통과한_척한다"]);
}

#[test]
fn 잠금_파일과_설정_파일이_다르면_실패한다() {
    let project = Project::new();
    project.add_unit("blocks", "second", &with_test(PLAIN_CODE));
    project.edit("Cargo.lock", "\n[[package]]\nname = \"second\"\nversion = \"0.1.0\"\n", ""); // 잠금 파일에서만 빠진 부품
    let before = project.read("Cargo.lock");
    let output = project.fails("시험", &["Cargo.toml 과 Cargo.lock 이 서로 맞지 않습니다."]);
    // 검사 도구가 돌지 못한 것도 「점검 못 함」이 아니라 실패다.
    assert!(output.contains("❌ 패턴") && output.contains("패턴 검사 도구(clippy)를 실행하지 못했습니다."), "{output}");
    // 점검은 잠금 파일을 고쳐 쓰지 않는다.
    assert_eq!(project.read("Cargo.lock"), before);
}

// ---- 금지 연결 ----

#[test]
fn 금지_사항에_시험이_없으면_실패한다() {
    let project = Project::new();
    project.edit(BLOCK, "fn n2_", "fn 둘째_");
    project.fails("금지 연결", &["blocks/sample: 금지 사항 [N2] 을 확인하는 시험이 없습니다. 시험 이름이 n2_ 로 시작해야 합니다."]);
}

#[test]
fn 확인하는_줄이_없는_시험은_실패한다() {
    let project = Project::new();
    project.edit(BLOCK, "assert!(shout(\"a\\nb\").is_err());", "let _ = shout(\"ab\");");
    project.fails("금지 연결", &["blocks/sample: 금지 사항 [N2] 의 시험에 확인하는 줄(assert…)이 없습니다."]);
    // 어디에도 이어지지 않은 파일에 같은 이름의 함수를 두어서 확인하는 줄이 있는 것처럼 꾸밀 수 없다.
    project.write("blocks/sample/src/unused.rs", "fn n2_줄바꿈이_든_글자는_받지_않는다() {\n    assert!(true);\n}\n");
    project.fails("금지 연결", &["blocks/sample: 금지 사항의 시험 「n2_줄바꿈이_든_글자는_받지_않는다」 이 두 군데에 있습니다."]);
}

#[test]
fn 안쪽_함수의_확인과_unwrap_이라는_필드는_확인하는_줄로_치지_않는다() {
    for body in [
        "fn _도우미() {\n            assert!(shout(\"\").is_err());\n        }\n        let _ = shout(\"ab\");",
        "struct S {\n            unwrap: i32,\n        }\n        let s = S { unwrap: 1 };\n        let _ = (s.unwrap, shout(\"ab\"));",
    ] {
        let project = Project::new();
        project.edit(BLOCK, "assert!(shout(\"a\\nb\").is_err());", body);
        project.fails("금지 연결", &["blocks/sample: 금지 사항 [N2] 의 시험에 확인하는 줄(assert…)이 없습니다."]);
    }
}

#[test]
fn unwrap_을_부르는_줄과_클로저_안의_확인은_확인하는_줄이다() {
    for body in ["let _ = shout(\"a\\nb\").unwrap_err();", "let check = |text: &str| assert!(shout(text).is_err());\n        check(\"a\\nb\");"] {
        let project = Project::new();
        project.edit(BLOCK, "assert!(shout(\"a\\nb\").is_err());", body);
        project.passes();
    }
}

#[test]
fn 금지_사항_본문에_적은_다른_번호는_금지_사항으로_치지_않는다() {
    let project = Project::new();
    project.edit(CARD, "- [N1] 빈 글자는 받지 않는다.", "- [N1] 빈 글자는 받지 않는다([N9]와 [N2]도 함께 본다).");
    let output = project.passes();
    assert!(output.contains("✅ 금지 연결: 카드의 금지 사항 2개마다"), "{output}");
}

#[test]
fn 실행되지_않는_시험은_있는_것으로_치지_않는다() {
    let project = Project::new();
    project.edit(BLOCK, "    #[test]\n    fn n1_", "    fn n1_");
    project.fails("금지 연결", &["blocks/sample: 금지 사항 [N1] 의 시험이 실제로 실행되지 않았습니다."]);
}

// ---- 통로 ----

#[test]
fn 블록이_바깥을_건드리면_실패한다() {
    let project = Project::new();
    let peek = r#"use std::{fs, time::{Duration, Instant}};
use std as base;

pub fn peek() -> usize {
    println!("보는 중");
    let _ = (Instant::now(), Duration::ZERO, env!("HOME"));
    let _ = std::hash::RandomState::new();
    unsafe { std::hint::unreachable_unchecked() };
    fs::read("memo.txt").map_or(0, |bytes| bytes.len()) + base::process::id() as usize
}

pub fn count() -> usize {
    eprintln!("{}", include_str!("lib.rs"));
    std::env::args().count()
}

extern "C" {
    fn getpid() -> i32;
}

pub fn noisy() -> usize {
    print!("a");
    eprint!("b");
    dbg!(1);
    let _ = (include_bytes!("lib.rs"), option_env!("HOME"), std::sync::Mutex::new(1));
    (0..std::process::id()).count()
}
"#;
    project.write("blocks/sample/src/peek.rs", peek);
    project.edit(BLOCK, "pub fn shout", "pub mod peek;\n\npub fn shout");
    let expected = [
        "peek.rs:1 블록이 허용되지 않은 표준 라이브러리 std::fs 을 썼습니다.",
        "peek.rs:1 블록이 허용되지 않은 표준 라이브러리 std::time::Instant 을 썼습니다.",
        "peek.rs:2 블록이 표준 라이브러리를 경로 없이 이름으로만 썼습니다.",
        "peek.rs:5 블록이 println! 를 썼습니다.",
        "peek.rs:6 블록이 env! 를 썼습니다.",
        "peek.rs:7 블록이 RandomState 를 썼습니다.",
        "peek.rs:8 블록이 unsafe 를 썼습니다.",
        "peek.rs:8 블록이 허용되지 않은 표준 라이브러리 std::hint::unreachable_unchecked 을 썼습니다.",
        "peek.rs:13 블록이 eprintln! 를 썼습니다.",
        "peek.rs:13 블록이 include_str! 를 썼습니다.",
        // 명령줄 인자는 조립 파일만 읽는다.
        "peek.rs:14 블록이 허용되지 않은 표준 라이브러리 std::env::args 을 썼습니다.",
        "peek.rs:17 블록이 extern 를 썼습니다.",
        "peek.rs:22 블록이 print! 를 썼습니다.",
        "peek.rs:23 블록이 eprint! 를 썼습니다.",
        "peek.rs:24 블록이 dbg! 를 썼습니다.",
        "peek.rs:25 블록이 include_bytes! 를 썼습니다.",
        "peek.rs:25 블록이 option_env! 를 썼습니다.",
        // sync 는 이름 몇 개만 허용한다. 범위(..) 뒤에 온 경로도 본다.
        "peek.rs:25 블록이 허용되지 않은 표준 라이브러리 std::sync::Mutex::new 을 썼습니다.",
        "peek.rs:26 블록이 허용되지 않은 표준 라이브러리 std::process::id 을 썼습니다.",
    ];
    let output = project.fails("통로", &expected);
    assert!(!output.contains("std::time::Duration") && !output.contains("std::hash::RandomState 을"), "{output}");
}

#[test]
fn 블록과_통로는_서로를_잇지_못하고_승인되지_않은_외부_부품도_쓰지_못한다() {
    let project = Project::new();
    project.add_unit("doors", "home", &with_test(DOOR_CODE));
    project.add_unit("blocks", "second", &with_test(PLAIN_CODE));
    let deps = "\n[dependencies]\nsample = { path = \"../sample\" }\nhome = { path = \"../../doors/home\" }\nserde = \"1\"\nrenamed = { package = \"serde\", version = \"1\" }\n";
    project.append("blocks/second/Cargo.toml", deps);
    // 조립 파일에서 쓰는 꼴을 그대로 옮겨 적어도, 잇는 일은 조립에서만 한다.
    project.append("doors/home/Cargo.toml", "\n[dev-dependencies]\nserde = { version = \"1\", features = [\"derive\"] }\nsample = { path = \"../blocks/sample\" }\n");
    let expected = [
        "blocks/second/Cargo.toml: 「sample」 을 받을 수 없는 방식으로 적었습니다.",
        "blocks/second/Cargo.toml: 「home」 을 받을 수 없는 방식으로 적었습니다.",
        "blocks/second/Cargo.toml: 「renamed」 을 받을 수 없는 방식으로 적었습니다.",
        "doors/home/Cargo.toml: 「sample」 을 받을 수 없는 방식으로 적었습니다.",
        "blocks/second/Cargo.toml: 승인되지 않은 외부 부품 「serde」 입니다.",
        "doors/home/Cargo.toml: 승인되지 않은 외부 부품 「serde」 입니다.",
    ];
    let output = project.fails("통로", &expected);
    assert!(output.contains("외부 부품 1개: serde\n"), "{output}");
    // 통로용으로 승인한 부품은 통로만 쓸 수 있다.
    project.edit("Cargo.toml", "door-crates = []", "door-crates = [\"serde\"]");
    let output = project.fails("통로", &expected[..5]);
    assert!(!output.contains(expected[5]), "{output}");
    // 블록과 조립용으로 승인한 부품은 어디서나 쓸 수 있다.
    project.edit("Cargo.toml", "extra-crates = []", "extra-crates = [\"serde\"]");
    let output = project.fails("통로", &expected[..4]);
    assert!(!output.contains(expected[4]) && output.contains("예외 1개(블록과 조립에 허용한 외부 부품 1개, 끈 패턴 검사 0개)"), "{output}");
}

#[test]
fn 조립_파일에_판단이_있으면_실패한다() {
    let project = Project::new();
    let new = "    let words: Vec<&str> = text.split(' ').filter(|word| !word.is_empty()).collect();\n    if words.is_empty() && text.is_empty() {\n        return Err(\"없음\".to_string());\n    }\n    println!";
    project.edit(APP, "    println!", new);
    project.fails("통로", &["app/src/main.rs:3 조립 파일에 판단", "app/src/main.rs:4 조립 파일에 판단"]);
}

#[test]
fn 조립_파일이_바깥을_직접_건드리면_실패한다() {
    let project = Project::new();
    project.edit(APP, "std::env::args().skip(1).collect::<Vec<_>>().join(\" \")", "std::env::var(\"NAME\").unwrap_or_default()");
    project.edit(APP, "    println!", "    let _ = (env!(\"HOME\"), unsafe { std::mem::zeroed::<u8>() });\n    println!");
    let expected = [
        "app/src/main.rs:2 조립 파일이 허용되지 않은 표준 라이브러리 std::env::var 을 썼습니다.",
        "app/src/main.rs:3 조립 파일이 env! 를 썼습니다.",
        "app/src/main.rs:3 조립 파일이 unsafe 를 썼습니다.",
    ];
    project.fails("통로", &expected);
}

#[test]
fn 다른_자리의_파일을_코드로_끌어올_수_없다() {
    let project = Project::new();
    project.add_unit("doors", "home", &with_test(DOOR_CODE));
    project.write("notes/extra.txt", "pub fn extra() -> u8 {\n    1\n}\n");
    project.edit("doors/home/src/lib.rs", "pub fn run", "#[path = \"../../../notes/extra.txt\"]\npub mod extra;\n\npub fn run");
    project.write("blocks/sample/tests/pull.rs", "include!(\"../../../notes/extra.txt\");\n\n#[test]\nfn 끌어온_코드() {\n    assert_eq!(extra(), 1);\n}\n");
    let expected = [
        "doors/home/src/lib.rs:1 다른 자리의 파일을 코드로 끌어왔습니다(#[path]).",
        "blocks/sample/tests/pull.rs:1 다른 자리의 파일을 코드로 끌어왔습니다(include!).",
    ];
    project.fails("통로", &expected);
}

// ---- 패턴 ----

#[test]
fn 위험한_패턴은_코드_안의_표시로_끌_수_없다() {
    let project = Project::new();
    let new = "#[allow(clippy::todo)]\npub fn later() {\n    todo!()\n}\n\n#[warn(warnings, clippy::dbg_macro)]\npub fn quiet() {\n    let spare = 1;\n    dbg!(3);\n}\n\npub fn sooner() -> u8 {\n    let extra = 1;\n    dbg!(2);\n    unsafe { std::mem::zeroed::<u8>() };\n    unimplemented!();\n    todo!()\n}\n\npub fn shout";
    project.edit(BLOCK, "pub fn shout", new);
    // 컴파일러의 경고도 실패로 보고, 기본으로는 꺼져 있는 검사 몇 개를 더 본다.
    let expected = [
        "blocks/sample/src/lib.rs:1 검사를 끄는 표시(allow·expect·warn)가 있습니다: clippy::todo.",
        // warn 은 실패로 올려 둔 검사를 경고로 되돌려서, 그 아래의 위반이 드러나지 않게 한다.
        "blocks/sample/src/lib.rs:6 검사를 끄는 표시(allow·expect·warn)가 있습니다: warnings.",
        "blocks/sample/src/lib.rs:6 검사를 끄는 표시(allow·expect·warn)가 있습니다: clippy::dbg_macro.",
        "blocks/sample/src/lib.rs:13: unused variable: `extra` [unused_variables]",
        "blocks/sample/src/lib.rs:14: the `dbg!` macro is intended as a debugging tool [clippy::dbg_macro]",
        "blocks/sample/src/lib.rs:15: usage of an `unsafe` block [unsafe_code]",
        "blocks/sample/src/lib.rs:16: `unimplemented` should not be present in production code [clippy::unimplemented]",
        "blocks/sample/src/lib.rs:17: `todo` should not be present in production code [clippy::todo]",
    ];
    let output = project.fails("패턴", &expected);
    for hidden in ["lib.rs:3:", "lib.rs:8:", "lib.rs:9:"] {
        assert!(!output.contains(hidden), "{output}");
    }
}

#[test]
fn 위험한_패턴은_설정_파일로_끌_수_없다() {
    let project = Project::new();
    project.edit(BLOCK, "pub fn shout", "pub fn sooner() {\n    todo!()\n}\n\npub fn shout");
    project.append("blocks/sample/Cargo.toml", "rust-version = \"1.60\"\n\n[lints.clippy]\ntodo = \"allow\"\n");
    project.edit("Cargo.toml", "extra-crates = []", "extra-crates = []\nextra-crate = [\"serde\"]"); // 이름을 잘못 적은 항목
    project.append("Cargo.toml", "\n[workspace.metadata.other]\nlevel = 1\n\n[workspace.lints.rust]\nwarnings = \"allow\"\n\n[profile.dev]\ndebug-assertions = false\n");
    for name in ["clippy.toml", "blocks/.clippy.toml", ".cargo/config.toml", "blocks/sample/.cargo/config.toml", "blocks/rust-toolchain.toml", "app/rust-toolchain"] {
        project.write(name, "");
    }
    // 맨 위의 것은 줄 머리부터 견준다. 그러지 않으면 아래 폴더의 같은 이름이 낸 줄에도 맞는다.
    let expected = [
        "\n     clippy.toml: 빌드와 검사를 바꾸는 설정 파일은 프로젝트에 둘 수 없습니다.",
        "blocks/.clippy.toml: 빌드와",
        "\n     .cargo: 빌드와",
        "blocks/sample/.cargo: 빌드와",
        "blocks/rust-toolchain.toml: 빌드와",
        "app/rust-toolchain: 빌드와",
        "blocks/sample/Cargo.toml: 「lints」 표는 둘 수 없습니다.",
        "blocks/sample/Cargo.toml: [package] 에 「rust-version」 는 둘 수 없습니다.",
        "Cargo.toml: [workspace] 에 「lints」 는 둘 수 없습니다.",
        "Cargo.toml: 맨 위에 「profile」 는 둘 수 없습니다.",
        "Cargo.toml: [workspace.metadata] 에 「other」 는 둘 수 없습니다.",
        "Cargo.toml: [workspace.metadata.blockkit] 에 「extra-crate」 는 둘 수 없습니다.",
    ];
    project.fails("패턴", &expected);
}

#[test]
fn 크레이트_설정은_정해진_항목만_받는다() {
    let project = Project::new();
    project.edit("blocks/sample/Cargo.toml", "name = \"sample\"", "name = \"sample\"\nbuild = \"src/lib.rs\"\nautotests = false");
    project.edit("blocks/sample/Cargo.toml", "edition = \"2024\"", "edition = \"2021\"");
    project.append("blocks/sample/Cargo.toml", "\n[lib]\ntest = false\n\n[features]\nextra = []\n");
    project.edit("Cargo.toml", "members = [\"app\"]", "members = [\"app\", \"blocks/*\"]\ndefault-members = [\"app\"]");
    let expected = [
        "blocks/sample/Cargo.toml: [package] 에 「build」 는 둘 수 없습니다.",
        "blocks/sample/Cargo.toml: [package] 에 「autotests」 는 둘 수 없습니다.",
        "blocks/sample/Cargo.toml: [package] 의 edition 은 \"2024\" 로 적습니다.",
        "blocks/sample/Cargo.toml: 「lib」 표는 둘 수 없습니다.",
        "blocks/sample/Cargo.toml: 「features」 표는 둘 수 없습니다.",
        "Cargo.toml: [workspace] 에 「default-members」 는 둘 수 없습니다.",
        "Cargo.toml: [workspace] 의 members 에는 \"app\" 하나만 적습니다.",
    ];
    project.fails("패턴", &expected);
    project.write("blocks/sample/Cargo.toml", "[package]\nname = \"other\"\nversion = \"0.1.0\"\nedition = \"2024\"\n");
    project.fails("패턴", &["blocks/sample/Cargo.toml: [package] 의 name 은 폴더 이름과 같은 \"sample\" 이어야 합니다."]);
    project.write("blocks/sample/Cargo.toml", "[package\n");
    project.fails("패턴", &["blocks/sample/Cargo.toml: 형식이 틀려서 읽지 못했습니다."]);
}

#[test]
fn 패턴_예외는_있는_파일_하나에_검사_이름_하나씩만_적을_수_있다() {
    let project = Project::new();
    project.edit(BLOCK, "pub fn shout", "#[allow(clippy::todo)]\npub fn later() {\n    todo!()\n}\n\npub fn shout");
    project.write("*", ""); // 이름이 무늬처럼 생긴 파일이 실제로 있어도 무늬는 받지 않는다.
    // 검사 묶음, 쉼표로 이어 붙인 값, 폴더 무늬, 어느 폴더에나 맞는 파일 이름, 모든 파일
    let widened = [
        "\"blocks/sample/src/lib.rs\" = [\"clippy::all\"]",
        "\"blocks/sample/src/lib.rs\" = [\"warnings\"]",
        "\"blocks/sample/src/lib.rs\" = [\"clippy::todo,dead_code\"]",
        "\"blocks/**\" = [\"clippy::todo\"]",
        "\"lib.rs\" = [\"clippy::todo\"]",
        "\"*\" = [\"clippy::todo\"]",
    ];
    for line in widened {
        project.write("Cargo.toml", &format!("{ROOT_MANIFEST}{line}\n"));
        project.fails("패턴", &["pattern-exceptions 의"]);
    }
    project.write("Cargo.toml", &format!("{ROOT_MANIFEST}\"blocks/sample/src/lib.rs\" = [\"clippy::todo\"]\n"));
    assert!(project.passes().contains("예외 1개(블록과 조립에 허용한 외부 부품 0개, 끈 패턴 검사 1개)"));
    // 예외를 적은 파일이 다른 파일을 열면, 검사를 끄는 표시가 그 파일에까지 걸릴 수 있다.
    project.write("blocks/sample/src/rules.rs", "pub fn clean() {}\n");
    project.edit(BLOCK, "#[allow", "pub mod rules;\n\n#[allow");
    project.fails("패턴", &["blocks/sample/src/lib.rs: 검사를 끄는 표시가 있는 파일에는 다른 파일을 여는 「mod 이름;」 선언을 둘 수 없습니다."]);
}

#[test]
fn 조건부_컴파일로_코드를_검사에서_뺄_수_없다() {
    let project = Project::new();
    let new = "#[cfg(not(clippy))]\npub fn later() {\n    todo!()\n}\n\n#[cfg_attr(clippy, allow(clippy::todo))]\npub fn sooner() {\n    todo!()\n}\n\n#[cfg(test)]\npub fn only_in_tests() {}\n\npub fn strict() -> bool {\n    cfg!(test) || cfg_select! { test => true, _ => false }\n}\n\npub fn shout";
    project.edit(BLOCK, "pub fn shout", new);
    let expected = [
        "blocks/sample/src/lib.rs:1 조건에 따라 코드를 빼는 표시는 #[cfg(test)] 만 씁니다.",
        "blocks/sample/src/lib.rs:6 조건에 따라 코드를 빼는 표시는 #[cfg(test)] 만 씁니다.",
        "blocks/sample/src/lib.rs:6 검사를 끄는 표시(allow·expect·warn)가 있습니다: clippy::todo.",
        // 시험할 때만 다르게 도는 코드: 시험할 때만 금지 사항을 지키게 만들 수 있다.
        "blocks/sample/src/lib.rs:11 #[cfg(test)] 는 시험 모듈(mod 이름) 바로 앞에만 붙입니다.",
        "blocks/sample/src/lib.rs:15 cfg! 를 썼습니다.",
        "blocks/sample/src/lib.rs:15 cfg_select! 를 썼습니다.",
    ];
    let output = project.fails("패턴", &expected);
    // 시험 모듈 앞에 붙은 것은 그대로 받는다.
    assert_eq!(output.matches("바로 앞에만 붙입니다").count(), 1, "{output}");
}

// ---- 비밀 값 ----

#[test]
fn 비밀_값이_있으면_실패하고_값은_출력하지_않는다() {
    let project = Project::new();
    project.write("notes.txt", &format!("키: {}\n", secret()));
    project.write("data.txt", &format!("{}\n{}\n", "x".repeat(1_100_000), secret())); // 큰 파일
    let output = project.fails("비밀 값", &["notes.txt:1 AWS 키", "data.txt:2 AWS 키"]);
    assert!(!output.contains(&secret()), "{output}");
}

// ---- 숨기기와 꾸미기 ----

#[test]
fn 건너뛰는_폴더_이름으로_코드와_비밀_값을_숨길_수_없다() {
    let project = Project::new();
    for name in ["notes/target/steal.rs", "blocks/sample/target/extra.rs"] {
        project.write(name, "fn main() {}\n");
    }
    let hidden = ["notes/.git/key.txt", "notes/target/key.txt", "notes/.claude/worktrees/key.txt", "notes/Cargo.lock"];
    for name in hidden {
        project.write(name, &secret());
    }
    fs::write(project.root.join("blocks/sample/fast.dylib"), b"\0").unwrap(); // 읽을 수 없는 코드 파일
    let output = project.fails("카드", &["notes/target/steal.rs: 정해진 자리 밖", "blocks/sample/target/extra.rs: 정해진 자리 밖", "blocks/sample/fast.dylib: 컴파일된 코드 파일"]);
    assert!(output.contains("❌ 비밀 값: 4건"), "{output}");
    for name in hidden {
        assert!(output.contains(&format!("{name}:1 AWS 키")), "{output}");
    }
}

#[test]
fn 프로젝트에_링크를_두면_실패한다() {
    let project = Project::new();
    let outside = project.dir.join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("loose.rs"), "pub const VALUE: u8 = 1;\n").unwrap();
    symlink(&outside, project.root.join("blocks/sample/linked")).unwrap();
    symlink(outside.join("loose.rs"), project.root.join("blocks/sample/src/extra.rs")).unwrap();
    project.fails("카드", &["blocks/sample/linked: 링크입니다.", "blocks/sample/src/extra.rs: 링크입니다."]);
}

#[test]
fn 대소문자만_바꾼_이름으로_코드와_설정을_숨길_수_없다() {
    // 맥의 파일 시스템은 대소문자를 가리지 않아서, 빌드 도구는 이런 이름의 파일도 읽는다.
    let project = Project::new();
    project.write("blocks/sample/src/peek.RS", "pub fn peek() -> u32 {\n    std::process::id()\n}\n");
    project.write("notes/stray.Rs", "fn main() {}\n");
    project.write("Clippy.toml", "");
    fs::write(project.root.join("blocks/sample/fast.DyLib"), b"\0").unwrap();
    let output = project.fails("통로", &["blocks/sample/src/peek.RS:2 블록이 허용되지 않은 표준 라이브러리 std::process::id 을 썼습니다."]);
    for text in ["notes/stray.Rs: 정해진 자리 밖", "blocks/sample/fast.DyLib: 컴파일된 코드 파일", "Clippy.toml: 빌드와 검사를 바꾸는 설정 파일"] {
        assert!(output.contains(text), "{output}");
    }
}

#[test]
fn 파일_이름과_카드_글자로_결과_줄을_꾸며_낼_수_없다() {
    let project = Project::new();
    project.write("stray\n결과: 전부 통과\n.rs", "pub const VALUE: u8 = 1;\n");
    project.write("blocks/a\u{2028}결과: 전부 통과/CARD.md", UNIT_CARD); // 줄바꿈 글자는 \n 말고도 있다.
    project.edit(CARD, "글자를 대문자로 바꾼다.", "글자를\u{1b}[2K 대문자로 바꾼다.");
    let (code, output) = project.check();
    assert_eq!(code, 1, "{output}");
    let verdicts: Vec<&str> = output.split(['\n', '\u{2028}']).filter(|line| line.starts_with("결과:")).collect();
    assert_eq!(verdicts.len(), 1, "{output}");
    assert!(verdicts[0].contains("실패"), "{output}");
    assert!(!output.contains('\u{1b}'), "{output}");
    let reply = project.hook("{}");
    assert_eq!(reply["systemMessage"], format!("점검(./check) {}", verdicts[0]));
    assert_eq!(reply["decision"], "block");
}

#[test]
fn 시험과_도구에는_환경_변수가_넘어가지_않는다() {
    let project = Project::new();
    let code = "pub fn run() -> String {\n    std::env::var(\"OWNER_ENV\").unwrap_or_else(|_| \"없음\".to_string())\n}\n";
    project.add_unit("doors", "env", &with_test(code).replace("assert!(!run().is_empty());", "assert_eq!(run(), \"없음\");"));
    project.edit(BLOCK, "pub fn shout", "pub fn sooner() {\n    todo!()\n}\n\npub fn shout");
    // RUSTFLAGS 가 넘어가면 경고가 전부 꺼져서 패턴 검사가 통과해 버린다.
    let (code, output) = project.check_with(&["--offline"], "", &[("OWNER_ENV", "주인의 값"), ("RUSTFLAGS", "--cap-lints allow")]);
    assert_eq!(code, 1, "{output}");
    assert!(output.contains("✅ 시험: 5개 통과"), "{output}");
    assert!(output.contains("❌ 패턴") && output.contains("[clippy::todo]"), "{output}");
}

// ---- 장치 ----

#[test]
fn 훅이나_잠금_승인_규칙이_빠지면_실패한다() {
    let project = Project::new();
    let settings: Value = serde_json::from_str(&project.read(DEVICES)).unwrap();
    assert!(project.passes().contains("✅ 장치"));
    // 틀 자체의 파일은 승인을 묻는 것이 아니라 잠겨 있어야 한다. 승인만 물으면 「>」 덧쓰기가 묻지 않고 실행된다.
    let locked = [
        "Edit(/check)",
        "Edit(/tools/check/**)",
        "Edit(/.claude/settings.json)",
        "Edit(/rust-toolchain.toml)",
        "Edit(/rust-toolchain)",
        "Edit(/.cargo/**)",
        "Edit(/.codex/**)",
        "Edit(/.agents/hooks.json)",
    ];
    assert_eq!(settings["permissions"]["deny"], json!(locked));
    let with = |pointer: &str, value: Value| {
        let mut changed = settings.clone();
        *changed.pointer_mut(pointer).unwrap() = value;
        changed
    };
    let mut broken = vec![with("/hooks", json!({}))];
    // 명령은 그대로 두고, 막지 못하게 바꾼 훅: 결과를 기다리지 않게 하거나 시간 한도를 줄인다.
    for (key, value) in [("async", json!(true)), ("timeout", json!(5))] {
        let mut weakened = settings.pointer("/hooks/Stop/0/hooks/0").unwrap().clone();
        weakened[key] = value;
        broken.push(with("/hooks/Stop/0/hooks/0", weakened));
    }
    for kind in ["deny", "ask"] {
        let rules = settings["permissions"][kind].as_array().unwrap();
        for dropped in rules {
            let rest: Vec<&Value> = rules.iter().filter(|rule| *rule != dropped).collect();
            broken.push(with(&format!("/permissions/{kind}"), json!(rest)));
        }
    }
    // 형식이 틀린 값이 하나라도 섞이면 Claude Code 가 이 파일을 통째로 읽지 않아서, 훅과 승인 규칙이 적혀 있어도 돌지 않는다.
    let mut other = settings.clone();
    other["hooks"]["PreToolUse"] = json!([{"matcher": ["Edit", "Write"], "hooks": [{"type": "command", "command": "true"}]}]);
    broken.push(other);
    let mut other = settings.clone();
    other["cleanupPeriodDays"] = json!("many");
    broken.push(other);
    assert_eq!(broken.len(), 23);
    for changed in broken {
        project.write(DEVICES, &changed.to_string());
        project.fails("장치", &[".claude/settings.json: 틀이 정한 내용과 다릅니다."]);
    }
    project.write(DEVICES, "{");
    project.fails("장치", &[".claude/settings.json: JSON 형식이 틀렸습니다."]);
    project.remove(DEVICES);
    project.fails("장치", &[".claude/settings.json: 틀이 정한 내용과 다릅니다."]);
}

#[test]
fn 훅을_끄는_설정이_있으면_실패한다() {
    let project = Project::new();
    project.write(".claude/settings.local.json", "{\"permissions\": {\"allow\": [\"Bash(cargo test:*)\"]}}");
    project.passes();
    project.write(".claude/settings.local.json", "{\"disableAllHooks\": true}");
    project.fails("장치", &[".claude/settings.local.json: 훅을 전부 끄는 설정(disableAllHooks)이 있습니다."]);
}

#[test]
fn 다른_하네스의_훅_설정이_빠지거나_바뀌거나_꺼지면_실패한다() {
    let project = Project::new();
    project.write(".codex/config.toml", "model = \"gpt-6.2-sol\"\n[features]\nweb_search = false\n");
    project.passes();
    for (name, key) in [(".codex/hooks.json", "/hooks/Stop/0/hooks/0"), (".agents/hooks.json", "/blockkit/PreToolUse/0/hooks/0")] {
        let original = project.read(name);
        let settings: Value = serde_json::from_str(&original).unwrap();
        let mut weakened = settings.clone();
        weakened.pointer_mut(key).unwrap()["timeout"] = json!(5);
        let mut removed = settings.clone();
        removed.as_object_mut().unwrap().clear();
        for changed in [weakened.to_string(), removed.to_string(), "{".to_string()] {
            project.write(name, &changed);
            project.fails("장치", &[&format!("{name}: 틀이 정한 내용과 다릅니다.")]);
        }
        project.remove(name);
        project.fails("장치", &[&format!("{name}: 틀이 정한 내용과 다릅니다.")]);
        project.write(name, &original);
    }
    project.passes();
    for config in ["[features]\nhooks = false\n", "[features]\ncodex_hooks = false\n"] {
        project.write(".codex/config.toml", config);
        project.fails("장치", &[".codex/config.toml: 훅을 끄는 설정(features.hooks = false)이 있습니다."]);
    }
}

#[test]
fn 고정한_버전이_아닌_컴파일러가_돌면_실패한다() {
    let project = Project::new();
    // 다른 버전의 컴파일러가 PATH 의 앞에 있는 경우
    script(&project.dir.join("bin/rustc"), "#!/bin/sh\necho 'rustc 1.0.0 (fake 2015-05-15)'\n");
    let front = format!("{}:{}", project.dir.join("bin").display(), path());
    let (code, output) = project.check_with(&["--offline"], "", &[("PATH", &front)]);
    assert_eq!(code, 1, "{output}");
    assert!(output.contains("❌ 장치") && output.contains("실제로 도는 버전(1.0.0)이 다릅니다."), "{output}");
    project.write("rust-toolchain.toml", "[toolchain]\nprofile = \"minimal\"\n");
    project.fails("장치", &["rust-toolchain.toml: Rust 버전이 고정돼 있지 않습니다."]);
}

// ---- 의존성 ----

#[test]
fn 취약한_외부_부품은_실패하지만_훅은_막지_않는다() {
    let project = Project::new();
    project.add_unit("doors", "old", &with_test(PLAIN_CODE));
    // 고쳐진 버전이 없는 취약점(RUSTSEC-2022-0004)이 알려진 부품
    project.append("doors/old/Cargo.toml", "\n[dependencies]\nrustc-serialize = \"0.3\"\n");
    project.edit("Cargo.toml", "door-crates = []", "door-crates = [\"rustc-serialize\"]");
    if !project.lock(true) && !project.lock(false) {
        eprintln!("건너뜀: 인터넷이 없어서 취약한 부품을 넣어 볼 수 없다");
        return;
    }
    let (code, output) = project.check_with(&[], "", &[]);
    if output.contains("⚠️ 의존성") {
        eprintln!("건너뜀: 인터넷이 없어서 취약점 목록을 받을 수 없다");
        return;
    }
    assert_eq!(code, 1, "{output}");
    assert!(output.contains("외부 부품 1개: rustc-serialize\n"), "{output}");
    assert!(output.contains("❌ 의존성") && output.contains("rustc-serialize 0.3.") && output.contains("알려진 취약점 RUSTSEC-2022-0004"), "{output}");
    assert_eq!(output.matches('❌').count(), 1, "{output}");
    let (code, reply) = project.check_with(&["--hook"], "{}", &[]);
    let reply: Value = serde_json::from_str(&reply).unwrap();
    assert_eq!(code, 0);
    assert!(reply.get("decision").is_none() && reply["systemMessage"].as_str().unwrap().contains("실패 1개"), "{reply}");
}

#[test]
fn 의존성_점검은_도구의_답을_읽어서_통과와_실패와_점검_못_함을_가린다() {
    // 인터넷 없이도 돌게, 받은 인자를 적어 두고 정해 준 대로 답하는 가짜 도구를 PATH 의 앞에 둔다.
    let project = Project::new();
    project.write("tools/check/Cargo.lock", "version = 4\n");
    let front = format!("{}:{}", project.dir.join("bin").display(), path());
    let calls = project.dir.join("calls");
    let run = |answer: &str, args: &[&str]| {
        script(&project.dir.join("bin/cargo-audit"), &format!("#!/bin/sh\necho \"$*\" >> '{}'\n{answer}\n", calls.display()));
        project.check_with(args, "{}", &[("PATH", &front)])
    };
    // 취약점이 없으면 전부 통과다. 프로젝트의 부품과 점검 명령의 부품을 둘 다 본다.
    let (code, output) = run(r#"echo '{"vulnerabilities":{"list":[]}}'"#, &[]);
    assert_eq!(code, 0, "{output}");
    assert!(output.contains("✅ 의존성") && output.ends_with("\n결과: 전부 통과\n"), "{output}");
    assert_eq!(fs::read_to_string(&calls).unwrap(), "audit --json --file Cargo.lock\naudit --json --file tools/check/Cargo.lock --no-fetch\n");
    // 취약점이 있으면 실패지만, 방금 한 일과 무관할 수 있어서 훅은 막지 않는다.
    let found = r#"echo '{"vulnerabilities":{"list":[{"package":{"name":"old","version":"0.1.0"},"advisory":{"id":"RUSTSEC-0000-0000"}}]}}'
exit 1"#;
    let (code, output) = run(found, &[]);
    assert_eq!(code, 1, "{output}");
    assert!(output.contains("❌ 의존성: 취약점이 알려진 외부 부품이 있습니다.\n     old 0.1.0: 알려진 취약점 RUSTSEC-0000-0000\n"), "{output}");
    let (code, reply) = run(found, &["--hook"]);
    let reply: Value = serde_json::from_str(&reply).unwrap();
    assert!(code == 0 && reply.get("decision").is_none() && reply["systemMessage"].as_str().unwrap().contains("실패 1개"), "{reply}");
    // 목록을 받지 못했거나 답을 읽지 못하면 통과도 실패도 아니다.
    let unread = [
        (r#"echo "error: couldn't fetch advisory database: git operation failed" >&2; exit 1"#, "⚠️ 의존성: 점검하지 못했습니다: 인터넷 연결이 필요합니다."),
        ("echo '알 수 없는 답'; echo '도구가 죽었다' >&2; exit 3", "⚠️ 의존성: 점검하지 못했습니다: 도구가 죽었다"),
    ];
    for (answer, expected) in unread {
        let (code, output) = run(answer, &[]);
        assert_eq!(code, 2, "{output}");
        assert!(output.contains(expected), "{output}");
    }
}

#[test]
fn 의존성_점검이_멈춰도_나머지_결과는_나온다() {
    let project = Project::new();
    script(&project.dir.join("bin/cargo-audit"), "#!/bin/sh\nexec sleep 90\n");
    let front = format!("{}:{}", project.dir.join("bin").display(), path());
    let (code, output) = project.check_with(&[], "", &[("PATH", &front)]);
    assert_eq!(code, 2, "{output}");
    assert!(output.contains("⚠️ 의존성: 점검하지 못했습니다: 30초 안에 끝나지 않았습니다.") && output.contains("✅ 시험"), "{output}");
}

// ---- 점검이 제대로 돌지 못할 때 ----

#[test]
fn 예외를_적는_설정을_읽지_못하면_시작하지_못한_것으로_보고_훅도_막는다() {
    let project = Project::new();
    // 목록 안의 목록, 표가 와야 할 자리에 적은 글자, 같은 표를 두 번 적은 실수
    let broken = [
        ("extra-crates = []", "extra-crates = [[\"serde\"]]"),
        ("[workspace.metadata.blockkit.pattern-exceptions]", "pattern-exceptions = \"없음\""),
        ("[workspace]", "[workspace]\n[workspace]"),
    ];
    for (old, new) in broken {
        project.write("Cargo.toml", &ROOT_MANIFEST.replace(old, new));
        let (code, output) = project.check();
        assert_eq!(code, 1, "{output}");
        assert!(output.starts_with("❌ 점검: 시작하지 못했습니다.") && output.ends_with("결과: 실패 1개(점검을 시작하지 못했습니다)\n"), "{output}");
        let page = project.read("target/check.html");
        assert!(page.contains(r#"class="verdict fail"><h1>결과: 실패 1개(점검을 시작하지 못했습니다)</h1>"#) && page.contains("Cargo.toml"), "{page}");
        assert_eq!(project.hook("{}")["decision"], "block");
    }
    let (code, output) = output_of(Command::new(env!("CARGO_BIN_EXE_check")), "");
    assert_eq!(code, 1);
    assert!(output.starts_with("❌ 점검: 시작하지 못했습니다."), "{output}");
}

// ---- 훅 ----

#[test]
fn 훅은_실패가_없으면_한_줄만_알리고_실패일_때는_끝내지_못하게_막는다() {
    let project = Project::new();
    let reply = project.hook("{}");
    assert_eq!(reply, json!({"systemMessage": "점검(./check) 결과: 실패는 없지만 점검하지 못한 항목이 1개 있습니다."}));
    project.remove(CARD);
    let reply = project.hook("{\"stop_hook_active\": false}");
    assert_eq!(reply["decision"], "block");
    assert!(reply["reason"].as_str().unwrap().contains("❌ 카드"), "{reply}");
    // 이미 한 번 막았으면 다시 막지 않고 실패를 알린다.
    let reply = project.hook("{\"stop_hook_active\": true}");
    assert!(reply.get("decision").is_none() && reply["systemMessage"].as_str().unwrap().contains("결과: 실패 "), "{reply}");
    // 입력이 이상해도 올바른 답을 내고 실패를 막는다.
    assert_eq!(project.hook("")["decision"], "block");
}

#[test]
fn 끝나지_않는_시험은_시간_한도에서_실패로_끝난다() {
    let project = Project::new();
    project.write("blocks/sample/tests/slow.rs", "#[test]\nfn 끝나지_않는다() {\n    std::thread::sleep(std::time::Duration::from_secs(300));\n}\n");
    let started = std::time::Instant::now();
    let (code, output) = project.check_with(&["--offline", "--limit", "30"], "", &[]);
    assert_eq!(code, 1, "{output}");
    assert!(output.contains("실패: blocks/sample: 끝나지_않는다 (시간 안에 끝나지 않았습니다)"), "{output}");
    assert!(output.contains("패턴 검사가 시간 안에 끝나지 않았습니다."), "{output}");
    assert!(started.elapsed() < std::time::Duration::from_secs(45), "{output}");
}

#[test]
fn 시간이_남지_않으면_도구를_부르는_관문은_실패로_끝난다() {
    let project = Project::new();
    let (code, output) = project.check_with(&["--offline", "--limit", "0"], "", &[]);
    assert_eq!(code, 1, "{output}");
    for text in ["❌ 시험: 시험을 시간 안에 빌드하지 못했습니다.", "패턴 검사가 시간 안에 끝나지 않았습니다.", "실제로 도는 버전(알 수 없음)이 다릅니다."] {
        assert!(output.contains(text), "「{text}」 가 없다:\n{output}");
    }
}

#[test]
fn 멈춘_훅은_시간_한도에서_막는_답을_내고_끝난다() {
    // 훅에 넘겨 주는 입력이 끝나지 않는 경우. 훅이 시간 한도에 걸려 끊기면 막지도 알리지도 못한 채 지나간다.
    let mut command = Command::new(env!("CARGO_BIN_EXE_check"));
    let mut child = command.args(["--hook", "--limit", "1", "--root", "/"]).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    let held_open = child.stdin.take();
    let mut reply = String::new();
    std::io::Read::read_to_string(&mut child.stdout.take().unwrap(), &mut reply).unwrap();
    assert_eq!(child.wait().unwrap().code(), Some(0));
    drop(held_open);
    let reply: Value = serde_json::from_str(&reply).unwrap();
    assert_eq!(reply["decision"], "block");
    assert!(reply["reason"].as_str().unwrap().contains("시작하지 못했습니다. 시간 안에 끝나지 않았습니다."), "{reply}");
}

#[test]
fn 멈춘_점검은_그림도_실패_페이지로_바꾼다() {
    let project = Project::new();
    project.passes();
    assert!(project.read("target/check.html").contains(r#"class="verdict skip""#));
    let mut command = Command::new(env!("CARGO_BIN_EXE_check"));
    let mut child = command.args(["--hook", "--limit", "1", "--root"]).arg(&project.root).env("PATH", path()).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    let held_open = child.stdin.take();
    assert_eq!(child.wait().unwrap().code(), Some(0));
    drop(held_open);
    let page = project.read("target/check.html");
    assert!(page.contains(r#"class="verdict fail""#) && page.contains(r#"<svg class="roof open""#), "{page}");
    assert!(page.contains("시간 안에 끝나지 않았습니다."), "{page}");
}

// ---- 실행기(check) ----

/// 실행기를 가짜 cargo 와 함께 임시 폴더에 놓는다. 가짜 cargo 는 받은 것을 적어 두고, 정해 준 대로 답한다.
struct Launcher {
    dir: PathBuf,
}

impl Drop for Launcher {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

impl Launcher {
    fn new(answer: &str) -> Launcher {
        let launcher = Launcher { dir: temp_dir() };
        fs::create_dir_all(launcher.root()).unwrap();
        fs::copy(real("check"), launcher.root().join("check")).unwrap();
        let record = "echo \"$*\" >> \"$HOME/calls\"\nprintf '%s\\n' \"$@\" > \"$HOME/args\"\npwd > \"$HOME/cwd\"\nprintf '%s' \"$PATH\" > \"$HOME/path\"\ncat > \"$HOME/stdin\"\n";
        script(&launcher.dir.join("bin/cargo"), &format!("#!/bin/sh\n{record}{answer}\n"));
        launcher
    }

    fn root(&self) -> PathBuf {
        self.dir.join("my project")
    }

    /// 다른 폴더에서, 실행 권한에 기대지 않고 부른다.
    fn run(&self, args: &[&str], stdin: &str) -> (i32, String) {
        self.run_with_path(args, stdin, &format!("{}:/usr/bin:/bin", self.dir.join("bin").display()))
    }

    fn run_with_path(&self, args: &[&str], stdin: &str, path: &str) -> (i32, String) {
        let mut command = Command::new("/bin/sh");
        command.arg(self.root().join("check")).args(args).current_dir(&self.dir).env_clear().env("HOME", &self.dir).env("PATH", path).env("TMPDIR", &self.dir);
        // 표준 오류도 닫힐 때까지 읽어서 stderr 파일에 남긴다. 남은 프로세스가 출력을 쥐고 있으면 여기서 기다리게 된다.
        let mut child = command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
        child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
        let done = child.wait_with_output().unwrap();
        fs::write(self.dir.join("stderr"), &done.stderr).unwrap();
        (done.status.code().unwrap_or(-1), String::from_utf8_lossy(&done.stdout).into_owned())
    }

    fn recorded(&self, name: &str) -> String {
        fs::read_to_string(self.dir.join(name)).unwrap_or_default()
    }
}

#[test]
fn 실행기는_점검_명령의_결과와_종료_코드를_그대로_전한다() {
    let launcher = Launcher::new("printf '블록 0개\\n\\n결과: 실패는 없지만 점검하지 못한 항목이 1개 있습니다.\\n'\nexit 2");
    let (code, output) = launcher.run(&["--offline"], "");
    assert_eq!((code, output.as_str()), (2, "블록 0개\n\n결과: 실패는 없지만 점검하지 못한 항목이 1개 있습니다.\n"));
    // 어디서 불려도 프로젝트 맨 위에서, 점검 명령 자신의 설정 파일로 빌드해서 돌린다.
    let root = launcher.recorded("cwd").trim().to_string();
    assert!(root.ends_with("/my project"), "{root}");
    assert_eq!(launcher.recorded("args"), format!("run\n--locked\n--quiet\n--manifest-path\ntools/check/Cargo.toml\n--\n--root\n{root}\n--offline\n"));
    // 터미널에서는 사람이 보고 있으므로 빌드를 따로 재지 않고 바로 돌린다.
    let tool = "--locked --quiet --manifest-path tools/check/Cargo.toml";
    assert_eq!(launcher.recorded("calls"), format!("run {tool} -- --root {root} --offline\n"));
    // 훅에서는 받은 입력을 그대로 넘기고, JSON 한 줄을 그대로 전한다. 점검 명령은 시간을 재면서 먼저 빌드한다.
    let launcher = Launcher::new("printf '{\"systemMessage\":\"점검\"}\\n'");
    let started = std::time::Instant::now();
    assert_eq!(launcher.run(&["--hook"], "{\"stop_hook_active\": false}"), (0, "{\"systemMessage\":\"점검\"}\n".to_string()));
    // 빌드를 감시하던 쪽이 훅의 출력을 쥐고 남아 있으면 훅이 30초 동안 끝나지 않은 것으로 보인다.
    assert!(started.elapsed() < std::time::Duration::from_secs(15));
    assert_eq!(launcher.recorded("stdin"), "{\"stop_hook_active\": false}");
    assert_eq!(launcher.recorded("calls"), format!("build {tool}\nrun {tool} -- --root {} --hook\n", launcher.recorded("cwd").trim()));
    // 감시를 멈출 때 셸이 찍는 줄이 훅의 표준 오류로 새지 않는다.
    assert_eq!(launcher.recorded("stderr"), "");
}

#[test]
fn 실행기는_훅에서_점검_명령의_빌드가_길어지면_막고_끝낸다() {
    // 훅은 300초에 끊긴다. 빌드가 끝나기를 마냥 기다리면 막지도 알리지도 못한 채 지나간다.
    // 실제 빌드 도구처럼 컴파일러를 따로 띄워 둔다. 빌드 도구가 끊긴 뒤에도 남는 프로세스가 훅의 출력을 쥐고 있으면 안 된다.
    let launcher = Launcher::new("case $1 in build) sleep 60 & wait ;; esac\nprintf '{\"systemMessage\":\"점검\"}\\n'");
    let started = std::time::Instant::now();
    let (code, reply) = launcher.run(&["--hook"], "{}");
    let reply: Value = serde_json::from_str(&reply).unwrap();
    assert_eq!((code, &reply["decision"]), (0, &json!("block")), "{reply}");
    assert!(reply["reason"].as_str().unwrap().contains("30초 안에 빌드하지 못했습니다."), "{reply}");
    assert!(started.elapsed() < std::time::Duration::from_secs(45));
    // 빌드가 끊겼으면 점검 명령을 돌리지 않는다.
    assert_eq!(launcher.recorded("calls"), "build --locked --quiet --manifest-path tools/check/Cargo.toml\n");
    assert_eq!(launcher.recorded("stderr"), "");
}

#[test]
fn 실행기는_찾는_자리에_없는_빌드_도구도_찾아서_그_폴더를_찾는_자리의_앞에_둔다() {
    // 훅은 PATH 가 짧은 환경에서 돌 수 있다. 빌드 도구가 컴파일러를 같은 자리에서 찾으려면 그 자리가 PATH 에 있어야 한다.
    let launcher = Launcher::new("printf '결과: 전부 통과\\n'");
    let home = launcher.dir.join(".cargo/bin");
    fs::create_dir_all(&home).unwrap();
    fs::rename(launcher.dir.join("bin/cargo"), home.join("cargo")).unwrap();
    assert_eq!(launcher.run_with_path(&[], "", "/usr/bin:/bin"), (0, "결과: 전부 통과\n".to_string()));
    assert_eq!(launcher.recorded("path"), format!("{}:/usr/bin:/bin", home.display()));
}

#[test]
fn 실행기는_점검_명령이_죽거나_끝까지_돌지_못하면_시작_실패로_본다() {
    // 빌드 실패, 아무 말 없이 끝남, 결과 줄 없이 끝남, 줄 머리가 아닌 자리에 섞인 「결과:」
    for answer in ["exit 101", "exit 0", "printf '블록 0개\\n'", "printf '카드 설명 결과: 전부 통과\\n'"] {
        let launcher = Launcher::new(answer);
        let (code, output) = launcher.run(&[], "");
        assert_eq!(code, 1, "{answer}: {output}");
        assert_eq!(output, "❌ 점검: 시작하지 못했습니다. 점검 명령(tools/check)을 빌드하거나 실행하지 못했습니다.\n\n결과: 실패 1개(점검을 시작하지 못했습니다)\n");
        // 훅에서는 0 으로 끝나면서 막는다. 이미 한 번 막았으면 알리기만 한다.
        let (code, reply) = launcher.run(&["--hook"], "{}");
        let reply: Value = serde_json::from_str(&reply).unwrap();
        assert_eq!((code, &reply["decision"]), (0, &json!("block")), "{answer}");
        assert_eq!(reply["systemMessage"], "점검(./check) 결과: 실패 1개(점검을 시작하지 못했습니다)");
        // 곧바로 실패한 빌드를 시간이 모자랐다고 알리지 않는다. 그 안내를 따라 다시 돌려도 풀리지 않기 때문이다.
        let reason = reply["reason"].as_str().unwrap();
        assert!(reason.contains("시작하지 못했습니다") && reason.contains("빌드하") && !reason.contains("30초"), "{reply}");
        for input in ["{\"stop_hook_active\":true}", "{\"hook_event_name\": \"Stop\", \"stop_hook_active\": true}"] {
            let (code, reply) = launcher.run(&["--hook"], input);
            let reply: Value = serde_json::from_str(&reply).unwrap();
            assert_eq!((code, reply.as_object().map(|fields| fields.len())), (0, Some(1)), "{reply}");
        }
    }
    // 훅에서 빌드가 실패했으면 점검 명령을 돌리지 않는다. 돌리면 한도 없이 다시 빌드하게 된다.
    let launcher = Launcher::new("exit 101");
    launcher.run(&["--hook"], "{}");
    assert_eq!(launcher.recorded("calls"), "build --locked --quiet --manifest-path tools/check/Cargo.toml\n");
    // 결과 줄은 있지만 종료 코드가 이상하면 실패로 끝낸다.
    let launcher = Launcher::new("printf '결과: 전부 통과\\n'\nexit 7");
    assert_eq!(launcher.run(&[], "").0, 1);
    // 훅인데 JSON 이 아닌 답
    let launcher = Launcher::new("printf '결과: 전부 통과\\n'");
    let (_, reply) = launcher.run(&["--hook"], "{}");
    assert_eq!(serde_json::from_str::<Value>(&reply).unwrap()["decision"], "block");
}

#[test]
fn 실행기는_점검을_시작하지_못하면_하네스마다_그_하네스의_말로_한_번만_막는다() {
    let launcher = Launcher::new("exit 101");
    let answer = |args: &[&str], input: &str| -> Value {
        let (code, reply) = launcher.run(args, input);
        assert_eq!(code, 0, "{reply}");
        serde_json::from_str(&reply).unwrap()
    };
    for (args, first, again) in [
        (vec!["--hook", "codex"], "{\"stop_hook_active\": false}", "{\"stop_hook_active\": true}"),
        (vec!["--hook", "hermes"], "{\"hook_event_name\": \"pre_verify\", \"extra\": {\"attempt\": 0}}", "{\"hook_event_name\": \"pre_verify\", \"extra\": {\"attempt\": 2}}"),
        (vec!["--hook", "hermes"], "{\"stop_hook_active\": true, \"extra\": {\"attempt\": 0}}", "{\"extra\": {\"attempt\":1}}"),
    ] {
        let reply = answer(&args, first);
        assert_eq!(reply["decision"], "block", "{args:?} {first}");
        assert!(reply["reason"].as_str().unwrap().contains("시작하지 못했습니다"), "{reply}");
        assert_eq!(answer(&args, again), json!({"systemMessage": "점검(./check) 결과: 실패 1개(점검을 시작하지 못했습니다)"}), "{args:?} {again}");
    }
    let antigravity = ["--hook", "antigravity"];
    let input = |id: &str| json!({"conversationId": id, "lastUserInput": "만들어 줘", "executionNum": 1, "terminationReason": "model_stop", "fullyIdle": true}).to_string();
    let reply = answer(&antigravity, &input("c-1"));
    assert_eq!(reply["decision"], "continue", "{reply}");
    assert!(reply["reason"].as_str().unwrap().contains("시작하지 못했습니다"), "{reply}");
    let stopped = json!({"decision": "stop", "reason": "점검(./check) 결과: 실패 1개(점검을 시작하지 못했습니다)"});
    assert_eq!(answer(&antigravity, &input("c-1")), stopped);
    assert_eq!(answer(&antigravity, &input("c-2"))["decision"], "continue");
    assert_eq!(answer(&antigravity, &input("c_4"))["decision"], "continue");
    for input in ["{}", "", "{\"conversationId\": \"../x\"}", "{\"conversationId\": \"c-3\", \"terminationReason\": \"error\", \"fullyIdle\": true}", "{\"conversationId\": \"c-3\", \"terminationReason\": \"model_stop\", \"fullyIdle\": false}"] {
        assert_eq!(answer(&antigravity, input), stopped, "{input}");
    }
    assert_eq!(answer(&antigravity, &input("c-3"))["decision"], "continue");
    for dialect in ["codex", "hermes", "antigravity"] {
        assert_eq!(launcher.run(&["--guard", dialect], "{\"tool_name\": \"Bash\"}"), (0, String::new()), "{dialect}");
    }
}

#[test]
fn 실행기가_점검을_시작하지_못하면_그림_페이지도_실패로_바꾸고_지킴이는_페이지를_쓰지_않는다() {
    let launcher = Launcher::new("exit 101");
    let page = launcher.root().join("target/check.html");
    assert_eq!(launcher.run(&["--guard", "codex"], "{}").0, 0);
    assert!(!page.exists());
    assert_eq!(launcher.run(&[], "").0, 1);
    let text = fs::read_to_string(&page).unwrap();
    assert!(text.contains("점검을 시작하지 못했습니다") && text.contains("빌드하거나 실행하지 못했습니다"), "{text}");
}

#[test]
fn 실행기는_지킴이의_답을_그대로_전하고_답이_없으면_아무것도_내지_않는다() {
    let deny = r#"{"decision":"deny","reason":"막음"}"#;
    let launcher = Launcher::new(&format!("printf '%s' '{deny}'"));
    assert_eq!(launcher.run(&["--guard", "antigravity"], "{}"), (0, format!("{deny}\n")));
    assert!(launcher.recorded("args").contains("--guard\nantigravity"), "{}", launcher.recorded("args"));
    assert_eq!(launcher.recorded("stdin"), "{}");
    let launcher = Launcher::new("exit 0");
    assert_eq!(launcher.run(&["--guard", "codex"], "{}"), (0, String::new()));
}

#[test]
fn 실행기는_빌드를_바꾸는_설정이_프로젝트에_있으면_빌드_도구를_부르지_않는다() {
    for (name, expected) in [(".cargo/config.toml", "프로젝트에 .cargo 가 있습니다."), ("rust-toolchain", "프로젝트에 rust-toolchain 파일이 있습니다.")] {
        let launcher = Launcher::new("printf '결과: 전부 통과\\n'");
        fs::create_dir_all(launcher.root().join(name).parent().unwrap()).unwrap();
        fs::write(launcher.root().join(name), "").unwrap();
        let (code, output) = launcher.run(&[], "");
        assert_eq!(code, 1, "{output}");
        assert!(output.starts_with(&format!("❌ 점검: 시작하지 못했습니다. {expected}")), "{output}");
        assert_eq!(serde_json::from_str::<Value>(&launcher.run(&["--hook"], "{}").1).unwrap()["decision"], "block");
        assert_eq!(launcher.recorded("args"), "");
    }
}
