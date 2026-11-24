//! 관문 여덟 개와, 관문이 보는 프로젝트의 모습.
//! 관문은 규칙을 지키려는 작성자의 실수를 잡는 장치다. 일부러 숨긴 코드를 막는 격리 장치가 아니다.

use crate::run::{self, Done, Failure};
use crate::scan;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const PASS: &str = "✅";
pub const FAIL: &str = "❌";
pub const SKIP: &str = "⚠️";

const SECTIONS: [&str; 4] = ["하는 일", "입력", "출력", "금지 사항"];
/// 이 자리에 있을 때만 건너뛰는 폴더. 같은 이름이라도 다른 자리에 있으면 본다(코드를 숨기는 자리가 되지 않게).
/// .claude/worktrees: Claude Code 가 만드는 작업 사본은 프로젝트 전체의 복사본이다. 사본은 그 안의 check 가 점검한다.
const SKIPPED: [&str; 4] = [".git", "target", "tools/check/target", ".claude/worktrees"];
/// 읽고 점검할 수 없는 코드 파일의 확장자
const COMPILED: [&str; 8] = ["rlib", "rmeta", "so", "dylib", "a", "o", "dll", "wasm"];

/// 답을 끝낼 때마다 점검을 돌리는 훅. sh 로 불러서 파일의 실행 권한에 기대지 않는다.
const HOOK_COMMAND: &str = "sh \"${CLAUDE_PROJECT_DIR}/check\" --hook";
/// 틀 자체의 파일. .claude/settings.json 의 permissions.deny 에 있어야 편집 도구와 터미널 명령(>, tee, sed, cp, mv, rm)이 이 파일을 고치지 못한다.
/// 승인만 물으면 「편집 자동 허용」 모드에서 「>」 덧쓰기가 묻지 않고 실행된다. 틀은 이 설정이 적용되지 않는 상위 폴더에서 연 세션에서 고친다.
/// rust-toolchain 과 .cargo 는 없어야 하는 파일이다. 생기면 점검 명령이 빌드되기 전에 컴파일러와 빌드 설정이 바뀐다.
const DENY_RULES: [&str; 8] = [
    "Edit(/check)",
    "Edit(/tools/check/**)",
    "Edit(/.claude/settings.json)",
    "Edit(/rust-toolchain.toml)",
    "Edit(/rust-toolchain)",
    "Edit(/.cargo/**)",
    "Edit(/.codex/**)",
    "Edit(/.agents/hooks.json)",
];
/// 주인 승인 없이 바뀌면 안 되는 것들. .claude/settings.json 의 permissions.ask 에 있어야 Claude Code 가 주인에게 묻는다.
/// Bash 규칙: 「편집 자동 허용」 모드는 rm·mv·cp 를 묻지 않고 돌려서, 위의 Edit 규칙을 돌아갈 수 있다.
// shortcut: 카드처럼 승인만 묻는 파일은 「편집 자동 허용」 모드의 「>」 덧쓰기를 막지 못한다, 필요해지면 샌드박스로 막는다.
const ASK_RULES: [&str; 10] = [
    "Edit(/Cargo.toml)",
    "Edit(/Cargo.lock)",
    "Edit(/AGENTS.md)",
    "Edit(/CLAUDE.md)",
    "Edit(/.claude/**)",
    "Edit(/blocks/**/CARD.md)",
    "Edit(/doors/**/CARD.md)",
    "Bash(rm *)",
    "Bash(mv *)",
    "Bash(cp *)",
];

fn harness_command(mode: &str) -> String {
    format!("sh -c 'd=$(pwd); while [ ! -f \"$d/check\" ] || [ ! -d \"$d/tools/check\" ]; do [ \"$d\" = / ] && exit 0; d=$(dirname \"$d\"); done; exec sh \"$d/check\" {mode}'")
}

fn harness_hooks() -> [(&'static str, Value); 2] {
    let hook = |mode: &str, timeout: u32| json!({"type": "command", "command": harness_command(mode), "timeout": timeout});
    [
        (".codex/hooks.json", json!({"hooks": {"Stop": [{"hooks": [hook("--hook codex", 300)]}], "PreToolUse": [{"hooks": [hook("--guard codex", 60)]}]}})),
        (".agents/hooks.json", json!({"blockkit": {"Stop": [hook("--hook antigravity", 300)], "PreToolUse": [{"matcher": ".*", "hooks": [hook("--guard antigravity", 60)]}]}})),
    ]
}

/// 블록은 계산만 한다. 그래서 표준 라이브러리에서 바깥(파일, 네트워크, 다른 프로그램, 환경 변수, 시계, 난수, 대기)과 닿지 않는 모듈만 허용한다.
const PURE_STD: [&str; 28] = [
    "array", "ascii", "borrow", "boxed", "cell", "char", "clone", "cmp", "collections", "convert", "default", "error", "f32", "f64", "fmt", "hash", "iter",
    "marker", "mem", "num", "ops", "option", "rc", "result", "slice", "str", "string", "vec",
];
/// 모듈 전체가 아니라 이 이름 하나만 허용하는 것. time 에는 시계가, sync 에는 대기가 같이 들어 있다.
const PURE_STD_ITEMS: [(&str, &str); 4] = [("time", "Duration"), ("sync", "Arc"), ("sync", "OnceLock"), ("sync", "LazyLock")];
/// 조립 파일이 더 쓸 수 있는 것: 명령줄 인자와 종료 코드.
const APP_STD_ITEMS: [(&str, &str); 2] = [("env", "args"), ("process", "ExitCode")];
/// 화면에 쓰는 매크로. 블록에서는 못 쓰고 조립 파일에서는 쓴다.
const SCREEN_MACROS: [&str; 5] = ["print", "println", "eprint", "eprintln", "dbg"];
/// 컴파일할 때 파일과 환경 변수를 읽어 오는 매크로
const OUTSIDE_MACROS: [&str; 4] = ["include_str", "include_bytes", "env", "option_env"];

/// 패턴 검사의 규칙은 여기에 고정한다. 경고를 전부 실패로 보고, 기본으로는 꺼져 있는 검사 몇 개를 더 켠다.
/// Cargo.toml 의 [lints], clippy.toml, .cargo/config.toml 로는 바꿀 수 없다(그런 설정이 있으면 이 관문이 실패한다).
const LINT_FLAGS: [&str; 10] = ["-D", "warnings", "-D", "unsafe_code", "-D", "clippy::todo", "-D", "clippy::unimplemented", "-D", "clippy::dbg_macro"];
/// 검사 하나가 아니라 여러 검사를 한꺼번에 가리키는 이름. 예외로 받으면 화면의 「예외 N개」보다 넓게 검사가 꺼진다.
const LINT_GROUPS: [&str; 23] = [
    "warnings",
    "unused",
    "future_incompatible",
    "nonstandard_style",
    "let_underscore",
    "keyword_idents",
    "refining_impl_trait",
    "deprecated_safe",
    "rust_2018_compatibility",
    "rust_2018_idioms",
    "rust_2021_compatibility",
    "rust_2024_compatibility",
    "unknown_or_malformed_diagnostic_attributes",
    "clippy::all",
    "clippy::cargo",
    "clippy::complexity",
    "clippy::correctness",
    "clippy::nursery",
    "clippy::pedantic",
    "clippy::perf",
    "clippy::restriction",
    "clippy::style",
    "clippy::suspicious",
];

// 관문마다 프로그램 하나에 주는 시간. 점검 전체의 한도는 main 이 정해서 넘겨준다.
const BUILD_LIMIT: Duration = Duration::from_secs(150);
const TEST_LIMIT: Duration = Duration::from_secs(60);
const LINT_LIMIT: Duration = Duration::from_secs(90);
const AUDIT_LIMIT: Duration = Duration::from_secs(30);

pub struct Gate {
    pub status: &'static str,
    pub summary: String,
    pub details: Vec<String>,
}

fn pass(summary: impl Into<String>) -> Gate {
    Gate { status: PASS, summary: summary.into(), details: Vec::new() }
}

fn fail(summary: impl Into<String>, details: Vec<String>) -> Gate {
    Gate { status: FAIL, summary: summary.into(), details }
}

fn skip(summary: impl Into<String>) -> Gate {
    Gate { status: SKIP, summary: summary.into(), details: Vec::new() }
}

fn counted(problems: Vec<String>, good: String) -> Gate {
    if problems.is_empty() { pass(good) } else { fail(format!("{}건", problems.len()), problems) }
}

/// 블록이나 통로 하나
pub struct Unit {
    pub kind: &'static str,
    pub name: String,
}

impl Unit {
    pub fn rel(&self) -> String {
        format!("{}/{}", self.kind, self.name)
    }
}

#[derive(PartialEq)]
enum Kind {
    File,
    Dir,
    Link,
}

/// 설정 파일(Cargo.toml)들을 읽어서 알아낸 것
#[derive(Default)]
struct Manifests {
    /// 외부 부품과 잇기 규칙을 어긴 것. 「통로」 관문이 알린다.
    deps: Vec<String>,
    /// 빌드와 검사를 바꾸는 설정. 「패턴」 관문이 알린다.
    config: Vec<String>,
    /// 가져다 쓰는 외부 부품의 이름
    externals: BTreeSet<String>,
    /// 조립(app)에 이어진 블록과 통로
    wired: BTreeSet<String>,
}

pub struct Project {
    root: PathBuf,
    /// 프로젝트 안의 모든 파일·폴더·링크의 경로(맨 위 폴더 기준)
    entries: Vec<(String, Kind)>,
    pub units: Vec<Unit>,
    pub extra: Vec<String>,
    pub exceptions: BTreeMap<String, Vec<String>>,
    manifests: Manifests,
    offline: bool,
    deadline: Instant,
}

fn strings(value: &toml::Value) -> Option<Vec<String>> {
    value.as_array()?.iter().map(|item| item.as_str().map(str::to_string)).collect()
}

fn sub<'a>(table: &'a toml::Table, key: &str) -> Option<&'a toml::Table> {
    table.get(key)?.as_table()
}

fn outside<'a>(table: Option<&'a toml::Table>, allowed: &[&str]) -> Vec<&'a String> {
    table.into_iter().flat_map(|table| table.keys()).filter(|key| !allowed.contains(&key.as_str())).collect()
}

/// 폴더를 돌며 모든 항목을 이름 순서로 모은다. 링크는 따라가지 않는다.
fn walk(root: &Path, rel: &str, out: &mut Vec<(String, Kind)>) {
    let Ok(listing) = fs::read_dir(root.join(rel)) else { return };
    let mut found: Vec<(String, fs::FileType)> =
        listing.flatten().filter_map(|entry| Some((entry.file_name().to_string_lossy().into_owned(), entry.file_type().ok()?))).collect();
    found.sort_by(|left, right| left.0.cmp(&right.0));
    for (name, kind) in found {
        let path = if rel.is_empty() { name } else { format!("{rel}/{name}") };
        if SKIPPED.contains(&path.as_str()) {
            continue;
        }
        if kind.is_symlink() {
            out.push((path, Kind::Link));
        } else if kind.is_dir() {
            out.push((path.clone(), Kind::Dir));
            walk(root, &path, out);
        } else {
            out.push((path, Kind::File));
        }
    }
}

/// 맨 위의 Cargo.toml 에는 [workspace] 와 점검의 예외만 둔다. [profile] 이나 [patch] 는 빌드를, [workspace.lints] 는 검사를 바꾼다.
fn root_manifest(manifest: &toml::Table, found: &mut Manifests) {
    let workspace = sub(manifest, "workspace");
    let metadata = workspace.and_then(|table| sub(table, "metadata"));
    let tables = [
        (Some(manifest), vec!["workspace"], "맨 위에"),
        (workspace, vec!["resolver", "members", "metadata"], "[workspace] 에"),
        (metadata, vec!["blockkit"], "[workspace.metadata] 에"),
        (metadata.and_then(|table| sub(table, "blockkit")), vec!["door-crates", "extra-crates", "pattern-exceptions"], "[workspace.metadata.blockkit] 에"),
    ];
    for (table, allowed, place) in tables {
        for key in outside(table, &allowed) {
            found.config.push(format!("Cargo.toml: {place} 「{key}」 는 둘 수 없습니다."));
        }
    }
    if workspace.and_then(|table| table.get("members")).and_then(strings) != Some(vec!["app".to_string()]) {
        let how = "블록과 통로는 app/Cargo.toml 의 [dependencies] 에 이어서 들입니다.";
        found.config.push(format!("Cargo.toml: [workspace] 의 members 에는 \"app\" 하나만 적습니다. {how}"));
    }
}

/// 블록·통로·조립의 Cargo.toml 은 정해진 표와 항목만 받는다. 그 밖의 항목(build, [lib], [features], [lints] 등)은 빌드되는 코드나 도는 시험, 검사를 바꾼다.
fn unit_manifest(root: &Path, dir: &str, name: &str, allowed: &[&str], units: &[Unit], app: bool, found: &mut Manifests) {
    let file = format!("{dir}/Cargo.toml");
    // 파일이 없는 경우는 카드 관문과 시험 관문이 알린다.
    let Ok(text) = fs::read_to_string(root.join(&file)) else { return };
    let Ok(manifest) = text.parse::<toml::Table>() else {
        found.config.push(format!("{file}: 형식이 틀려서 읽지 못했습니다."));
        return;
    };
    for key in outside(Some(&manifest), &["package", "dependencies", "dev-dependencies"]) {
        found.config.push(format!("{file}: 「{key}」 표는 둘 수 없습니다. [package], [dependencies], [dev-dependencies] 만 둡니다."));
    }
    let package = sub(&manifest, "package");
    for key in outside(package, &["name", "version", "edition", "publish", "description"]) {
        found.config.push(format!("{file}: [package] 에 「{key}」 는 둘 수 없습니다."));
    }
    let field = |key: &str| package.and_then(|table| table.get(key)).and_then(toml::Value::as_str);
    if field("name") != Some(name) {
        found.config.push(format!("{file}: [package] 의 name 은 폴더 이름과 같은 \"{name}\" 이어야 합니다."));
    }
    if field("edition") != Some("2024") {
        found.config.push(format!("{file}: [package] 의 edition 은 \"2024\" 로 적습니다."));
    }
    for section in ["dependencies", "dev-dependencies"] {
        for (dep, value) in sub(&manifest, section).into_iter().flatten() {
            let fields = value.as_table();
            let versioned = fields.is_some_and(|table| table.get("version").is_some_and(toml::Value::is_str) && outside(Some(table), &["version", "features", "default-features"]).is_empty());
            let link = fields.filter(|table| table.len() == 1).and_then(|table| table.get("path")).and_then(toml::Value::as_str);
            let unit = units.iter().find(|unit| app && unit.name == *dep && link == Some(format!("../{}", unit.rel()).as_str()));
            if let Some(unit) = unit {
                // 시험에만 이은([dev-dependencies]) 블록은 조립된 프로그램에 들어가지 않는다.
                if section == "dependencies" {
                    found.wired.insert(unit.rel());
                }
            } else if !value.is_str() && !versioned {
                let how = "외부 부품은 버전으로만 적고(path·git·package 는 쓰지 않습니다), 블록과 통로는 app/Cargo.toml 에서만 { path = \"../blocks/이름\" } 꼴로 잇습니다.";
                found.deps.push(format!("{file}: 「{dep}」 을 받을 수 없는 방식으로 적었습니다. {how}"));
            } else {
                found.externals.insert(dep.clone());
                if !allowed.contains(&dep.as_str()) {
                    let how = "맨 위 Cargo.toml 의 door-crates(통로) 나 extra-crates(블록과 조립) 에 주인의 승인을 받아 먼저 적습니다.";
                    found.deps.push(format!("{file}: 승인되지 않은 외부 부품 「{dep}」 입니다. {how}"));
                }
            }
        }
    }
}

impl Project {
    /// 프로젝트를 읽는다. 예외를 적는 설정부터 읽지 못하면 점검을 시작하지 못한 것이다.
    pub fn load(root: &Path, offline: bool, limit: Duration) -> Result<Project, String> {
        // 빌드 도구가 알려 주는 경로와 견줄 수 있게, 링크를 푼 실제 경로로 바꾼다.
        let root = fs::canonicalize(root).map_err(|error| format!("점검할 폴더를 열지 못했습니다({error})."))?;
        let text = fs::read_to_string(root.join("Cargo.toml")).map_err(|error| format!("맨 위의 Cargo.toml 을 읽지 못했습니다({error})."))?;
        let manifest: toml::Table = text.parse().map_err(|_| "맨 위의 Cargo.toml 의 형식이 틀렸습니다.".to_string())?;
        let settings = sub(&manifest, "workspace").and_then(|table| sub(table, "metadata")).and_then(|table| table.get("blockkit"));
        let list = |key: &str| match settings.and_then(|table| table.get(key)) {
            None => Ok(Vec::new()),
            Some(value) => strings(value).ok_or(format!("맨 위 Cargo.toml 의 {key} 는 글자의 목록이어야 합니다.")),
        };
        let (extra, door_crates) = (list("extra-crates")?, list("door-crates")?);
        let mut exceptions = BTreeMap::new();
        if let Some(value) = settings.and_then(|table| table.get("pattern-exceptions")) {
            let wrong = "맨 위 Cargo.toml 의 pattern-exceptions 는 「\"파일\" = [\"검사 이름\"]」 꼴의 표여야 합니다.";
            for (file, lints) in value.as_table().ok_or(wrong)? {
                exceptions.insert(file.clone(), strings(lints).ok_or(wrong)?);
            }
        }
        let mut entries = Vec::new();
        walk(&root, "", &mut entries);
        let mut units = Vec::new();
        for kind in ["blocks", "doors"] {
            for (rel, what) in &entries {
                if let Some(name) = rel.strip_prefix(kind).and_then(|rest| rest.strip_prefix('/'))
                    && *what == Kind::Dir
                    && !name.contains('/')
                {
                    units.push(Unit { kind, name: name.to_string() });
                }
            }
        }
        let mut manifests = Manifests::default();
        root_manifest(&manifest, &mut manifests);
        let pure: Vec<&str> = extra.iter().map(String::as_str).collect();
        let any: Vec<&str> = extra.iter().chain(&door_crates).map(String::as_str).collect();
        for unit in &units {
            unit_manifest(&root, &unit.rel(), &unit.name, if unit.kind == "doors" { &any } else { &pure }, &units, false, &mut manifests);
        }
        unit_manifest(&root, "app", "app", &pure, &units, true, &mut manifests);
        Ok(Project { root, entries, units, extra, exceptions, manifests, offline, deadline: Instant::now() + limit })
    }

    pub fn externals(&self) -> Vec<&str> {
        self.manifests.externals.iter().map(String::as_str).collect()
    }

    fn files(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().filter(|(_, kind)| *kind == Kind::File).map(|(rel, _)| rel.as_str())
    }

    fn is_file(&self, rel: &str) -> bool {
        self.files().any(|file| file == rel)
    }

    /// 코드가 놓이는 폴더: 블록과 통로, 그리고 조립.
    fn crates(&self) -> Vec<String> {
        self.units.iter().map(Unit::rel).chain(["app".to_string()]).collect()
    }

    /// `dir` 의 `folders`(src, tests) 안에 있는 Rust 파일
    fn sources(&self, dir: &str, folders: &[&str]) -> Vec<&str> {
        let inside = |file: &str, folder: &str| file.strip_prefix(dir).and_then(|rest| rest.strip_prefix('/')).and_then(|rest| rest.strip_prefix(folder)).is_some_and(|rest| rest.starts_with('/'));
        // 맥의 파일 시스템은 대소문자를 가리지 않아서 `mod peek;` 가 peek.RS 도 연다. 그래서 확장자는 소문자로 바꿔서 본다.
        self.files().filter(|file| file.to_ascii_lowercase().ends_with(".rs") && folders.iter().any(|folder| inside(file, folder))).collect()
    }

    fn tokens(&self, file: &str) -> Option<Vec<scan::Flat>> {
        scan::flatten(&fs::read_to_string(self.root.join(file)).ok()?).ok()
    }

    /// 카드의 「## 제목」 아래 줄들을 {제목: [줄]} 로 돌려준다. 카드가 없으면 비어 있다.
    pub fn card(&self, unit: &Unit) -> BTreeMap<String, Vec<String>> {
        let mut sections: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut current: Option<String> = None;
        // 다른 인코딩으로 저장된 카드는 제목을 찾지 못해서 카드 관문의 실패로 드러난다.
        let bytes = fs::read(self.root.join(unit.rel()).join("CARD.md")).unwrap_or_default();
        for line in String::from_utf8_lossy(&bytes).lines() {
            if let Some(title) = line.strip_prefix("## ") {
                // 같은 제목이 또 나와도 앞에 적힌 줄을 지우지 않는다.
                sections.entry(title.trim().to_string()).or_default();
                current = Some(title.trim().to_string());
            } else if let Some(title) = &current
                && !line.trim().is_empty()
            {
                sections.entry(title.clone()).or_default().push(line.trim().to_string());
            }
        }
        sections
    }

    /// 맨 위 폴더 아래의 `dir` 에서 프로그램을 실행한다. 점검 전체의 마감 시각을 넘기지 않는다.
    fn run(&self, program: &str, args: &[&str], dir: &str, limit: Duration) -> Result<Done, Failure> {
        run::run(program, args, &self.root.join(dir), self.deadline.min(Instant::now() + limit))
    }
}

/// 금지 사항 한 줄(`- [N1] 내용`)에서 번호를 꺼낸다. 형태가 다르면 없다.
fn forbidden_line(line: &str) -> Option<String> {
    let rest = line.strip_prefix("- [")?;
    let (number, text) = rest.split_once("] ")?;
    let digits = number.strip_prefix('N')?;
    (!digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()) && !text.is_empty()).then(|| number.to_string())
}

/// 카드의 금지 사항에 적힌 번호들
pub fn forbidden_ids(card: &BTreeMap<String, Vec<String>>) -> Vec<String> {
    card.get("금지 사항").into_iter().flatten().map(String::as_str).filter_map(forbidden_line).collect()
}

/// Rust 코드 파일을 둘 수 있는 자리인가.
fn code_place(parts: &[&str]) -> bool {
    matches!(
        parts,
        ["blocks" | "doors", _, "src" | "tests", _, ..] | ["app", "src", "main.rs"] | ["app", "tests", _, ..] | ["tools", "check", "src" | "tests", _, ..]
    )
}

pub fn cards(project: &Project) -> Gate {
    let mut problems = Vec::new();
    for unit in &project.units {
        let rel = unit.rel();
        if !unit.name.starts_with(|first: char| first.is_ascii_lowercase()) || !unit.name.chars().all(|letter| letter.is_ascii_lowercase() || letter.is_ascii_digit() || letter == '_') {
            problems.push(format!("{rel}: 폴더 이름은 영어 소문자로 시작하고, 영어 소문자·숫자·밑줄만 씁니다."));
        }
        if !project.is_file(&format!("{rel}/CARD.md")) {
            problems.push(format!("{rel}: CARD.md 가 없습니다."));
        } else {
            let card = project.card(unit);
            for title in SECTIONS {
                if card.get(title).is_none_or(Vec::is_empty) {
                    problems.push(format!("{rel}: 카드에 「## {title}」 항목이 없거나 비어 있습니다."));
                }
            }
            // 번호가 없거나 겹친 금지 사항은 시험과 이어지지 않은 채 지나가므로 여기서 막는다.
            let mut numbers = Vec::new();
            for line in card.get("금지 사항").into_iter().flatten() {
                match forbidden_line(line) {
                    Some(number) => numbers.push(number),
                    None => problems.push(format!("{rel}: 금지 사항의 줄은 「- [N1] 내용」 형태로 적습니다: 「{}」", line.chars().take(30).collect::<String>())),
                }
            }
            let repeated: BTreeSet<&String> = numbers.iter().filter(|number| numbers.iter().filter(|other| other == number).count() > 1).collect();
            for number in repeated {
                problems.push(format!("{rel}: 금지 사항 번호 [{number}] 이 겹칩니다."));
            }
        }
        for (file, what) in [("Cargo.toml", "설정 파일(Cargo.toml)이"), ("src/lib.rs", "코드 파일(src/lib.rs)이")] {
            if !project.is_file(&format!("{rel}/{file}")) {
                problems.push(format!("{rel}: {what} 없습니다."));
            }
        }
        if !project.manifests.wired.contains(&rel) {
            problems.push(format!("{rel}: app/Cargo.toml 의 [dependencies] 에 이어져 있지 않습니다. 이어지지 않은 블록과 통로는 조립된 프로그램에 들어가지 않습니다."));
        }
    }
    // 카드도 시험도 없는 자리로 코드가 새 나가는 것을 막는다. 맨 위의 숨김 폴더(.claude 등)는 보지 않는다.
    for (rel, kind) in &project.entries {
        if rel.starts_with('.') {
            continue;
        }
        let parts: Vec<&str> = rel.split('/').collect();
        let extension = parts.last().and_then(|name| name.rsplit_once('.')).map_or(String::new(), |(_, extension)| extension.to_ascii_lowercase());
        if *kind == Kind::Link {
            problems.push(format!("{rel}: 링크입니다. 링크 너머의 코드는 점검이 보지 못하므로 프로젝트에 링크를 두지 않습니다."));
        } else if *kind == Kind::File && COMPILED.contains(&extension.as_str()) {
            problems.push(format!("{rel}: 컴파일된 코드 파일입니다. 읽고 점검할 수 없는 코드는 프로젝트에 두지 않습니다."));
        } else if *kind == Kind::File && extension == "rs" && !code_place(&parts) {
            problems.push(format!("{rel}: 정해진 자리 밖에 놓인 코드 파일입니다. 코드는 블록·통로의 src/ 와 tests/, app/src/main.rs, app/tests/ 에 둡니다."));
        }
    }
    if project.sources("app", &["tests"]).is_empty() {
        problems.push("app/tests: 조립된 전체를 실제로 돌려 보는 시험이 없습니다.".to_string());
    }
    counted(problems, format!("블록·통로 {}개 모두 카드가 있고 조립에 이어져 있으며, 자리 밖에 놓인 코드가 없습니다.", project.units.len()))
}

/// 빌드 도구가 한 줄에 하나씩 내는 JSON 을 읽는다. JSON 이 아닌 줄은 버린다.
fn json_lines(text: &str) -> Vec<Value> {
    text.lines().filter(|line| line.starts_with('{')).filter_map(|line| serde_json::from_str(line).ok()).collect()
}

fn last_lines(text: &str, count: usize) -> Vec<String> {
    let lines: Vec<&str> = text.trim().lines().collect();
    lines[lines.len().saturating_sub(count)..].iter().map(|line| line.trim().to_string()).collect()
}

/// 컴파일러와 검사 도구가 낸 오류를 「파일:줄: 설명 [검사 이름]」 으로 모은다. 같은 코드가 두 번 컴파일돼서 겹친 것은 한 번만 적는다.
fn diagnostics(messages: &[Value]) -> Vec<String> {
    let mut found = Vec::new();
    for message in messages {
        let inner = &message["message"];
        if message["reason"] != "compiler-message" || inner["level"] != "error" {
            continue;
        }
        // 「오류가 몇 개 있어서 멈춘다」 같은 맺음말에는 위치가 없다.
        let Some(span) = inner["spans"].as_array().and_then(|spans| spans.iter().find(|span| span["is_primary"] == true)) else { continue };
        let code = inner["code"]["code"].as_str().map(|code| format!(" [{code}]")).unwrap_or_default();
        let line = format!("{}:{}: {}{code}", span["file_name"].as_str().unwrap_or("?"), span["line_start"], inner["message"].as_str().unwrap_or(""));
        if !found.contains(&line) {
            found.push(line);
        }
    }
    found
}

/// 시험의 긴 주소를 「어느 블록: 어떤 약속」 으로 줄인다. 시험 이름이 곧 깨진 약속의 설명이다.
fn pretty(unit: &str, name: &str) -> String {
    let short = name.rsplit("::").next().unwrap_or(name);
    if unit == "app" { format!("조립(app): {short}") } else { format!("{unit}: {short}") }
}

/// 시험을 돌린다. 실제로 돌아간 시험을 (블록·통로, 시험 이름) 으로 함께 돌려줘서 「금지 연결」 관문이 쓰게 한다.
pub fn tests(project: &Project) -> (Gate, Vec<(String, String)>) {
    let mut executed = Vec::new();
    let gate = run_tests(project, &mut executed);
    (gate, executed)
}

fn run_tests(project: &Project, executed: &mut Vec<(String, String)>) -> Gate {
    // --locked: Cargo.toml 과 Cargo.lock 이 어긋나면 멈춘다. 잠금 파일에 없는 부품은 쓸 수 없다.
    let built = match project.run("cargo", &["test", "--no-run", "--locked", "--workspace", "--message-format=json"], "", BUILD_LIMIT) {
        Ok(done) => done,
        Err(Failure::Timeout) => return fail("시험을 시간 안에 빌드하지 못했습니다.", Vec::new()),
        Err(Failure::Spawn(error)) => return fail(format!("빌드 도구(cargo)를 실행하지 못했습니다({error})."), Vec::new()),
    };
    let messages = json_lines(&built.stdout);
    if built.code != Some(0) {
        if built.stderr.contains("--locked") {
            return fail("Cargo.toml 과 Cargo.lock 이 서로 맞지 않습니다. 에이전트에게 「Cargo.lock 을 맞춰 줘」라고 말하면 됩니다.", Vec::new());
        }
        let found = diagnostics(&messages);
        return fail("시험을 빌드하지 못했습니다(설명은 도구가 낸 영어 원문입니다).", if found.is_empty() { last_lines(&built.stderr, 3) } else { found });
    }
    // 시험은 시험 실행 파일을 직접 불러서 하나씩 돌린다. 한꺼번에 돌리면 시험이 화면에 쓴 글자와 결과 줄을 가를 수 없다.
    let (mut listed, mut failed, mut dodged) = (Vec::new(), Vec::new(), Vec::new());
    for message in &messages {
        let (Some(program), Some(manifest)) = (message["executable"].as_str(), message["manifest_path"].as_str()) else { continue };
        if message["reason"] != "compiler-artifact" || message["profile"]["test"] != true {
            continue;
        }
        let Some(unit) = Path::new(manifest).parent().and_then(|dir| dir.strip_prefix(&project.root).ok()).map(|dir| dir.to_string_lossy().into_owned()) else { continue };
        let list = |flags: &[&str]| -> Option<Vec<String>> {
            let done = project.run(program, &[flags, &["--list", "--format", "terse"]].concat(), &unit, TEST_LIMIT).ok().filter(|done| done.code == Some(0))?;
            Some(done.stdout.lines().filter_map(|line| line.strip_suffix(": test")).map(str::to_string).collect())
        };
        let (Some(all), Some(ignored)) = (list(&[]), list(&["--ignored"])) else {
            failed.push(format!("{unit}: 시험 목록을 읽지 못했습니다."));
            continue;
        };
        for name in all {
            listed.push((unit.clone(), name.clone()));
            if ignored.contains(&name) {
                dodged.push(pretty(&unit, &name));
                continue;
            }
            // 통과는 종료 코드와 요약 줄을 둘 다 본다. 시험이 도중에 프로그램을 0 으로 끝내면 요약 줄이 나오지 않는다.
            // shortcut: 요약 줄을 직접 찍고 프로그램을 끝내는 시험은 통과로 읽힌다, 필요해지면 시험 실행기를 따로 둔다.
            let outcome = project.run(program, &["--exact", &name], &unit, TEST_LIMIT);
            if !matches!(&outcome, Ok(done) if done.code == Some(0) && done.stdout.lines().any(|line| line.starts_with("test result: ok. 1 passed;"))) {
                let late = if matches!(outcome, Err(Failure::Timeout)) { " (시간 안에 끝나지 않았습니다)" } else { "" };
                failed.push(format!("{}{late}", pretty(&unit, &name)));
            }
            executed.push((unit.clone(), name));
        }
    }
    // 소스에는 있는데 시험 목록에 없는 시험: 파일이 mod 선언으로 이어지지 않았거나, 조건에 걸려 빠진 것이다. 있는 줄 알았던 시험이 없는 것이다.
    let (mut missing, mut idle) = (Vec::new(), Vec::new());
    for dir in project.crates() {
        let mut sources: BTreeMap<String, Vec<&str>> = BTreeMap::new();
        for file in project.sources(&dir, &["src", "tests"]) {
            for function in project.tokens(file).iter().flat_map(|tokens| scan::functions(tokens)) {
                if function.test {
                    sources.entry(function.name).or_default().push(file);
                }
            }
        }
        for (name, files) in sources {
            let ran = listed.iter().filter(|(unit, listed)| *unit == dir && listed.rsplit("::").next() == Some(name.as_str())).count();
            if files.len() > ran {
                missing.push(format!("{} ({})", pretty(&dir, &name), files.join(", ")));
            }
        }
        if !executed.iter().any(|(unit, _)| *unit == dir) {
            idle.push(dir);
        }
    }
    let mut details: Vec<String> = failed.iter().map(|test| format!("실패: {test}")).collect();
    details.extend(dodged.iter().map(|test| format!("건너뜀: {test}")));
    details.extend(missing.iter().map(|test| format!("돌지 않음: {test}. 파일이 mod 선언으로 이어져 있는지 봅니다.")));
    details.extend(idle.iter().map(|dir| format!("{dir}: 실행된 시험이 없습니다.")));
    if details.is_empty() {
        return pass(format!("{}개 통과", executed.len()));
    }
    let summary = format!("{}개 가운데 실패 {}개, 건너뛴 시험 {}개, 돌지 않은 시험 {}개, 시험이 없는 곳 {}개", executed.len(), failed.len(), dodged.len(), missing.len(), idle.len());
    fail(summary, details)
}

pub fn links(project: &Project, executed: &[(String, String)]) -> Gate {
    let (mut problems, mut total) = (Vec::new(), 0);
    for unit in &project.units {
        let rel = unit.rel();
        // 번호 -> 그 번호의 시험마다 모자란 점. 빈 글자가 하나라도 있으면 제대로 된 시험이 있는 것이다.
        let mut reasons: BTreeMap<String, Vec<&str>> = BTreeMap::new();
        let mut names = Vec::new();
        for file in project.sources(&rel, &["src", "tests"]) {
            for function in project.tokens(file).iter().flat_map(|tokens| scan::functions(tokens)) {
                let Some(number) = scan::forbidden_number(&function.name) else { continue };
                // 실행됐는지는 이름으로 견준다. 같은 이름이 둘이면 확인하는 줄이 있는 쪽이 실제로 도는 쪽인지 알 수 없다.
                if names.contains(&function.name) {
                    problems.push(format!("{rel}: 금지 사항의 시험 「{}」 이 두 군데에 있습니다. 어느 쪽이 실제로 도는지 가릴 수 없으므로 하나만 둡니다.", function.name));
                }
                names.push(function.name.clone());
                let ran = executed.iter().any(|(dir, name)| *dir == rel && name.rsplit("::").next() == Some(function.name.as_str()));
                let reason = if !ran {
                    "의 시험이 실제로 실행되지 않았습니다. 시험에는 #[test] 를 붙이고, 건너뛰게(#[ignore]) 하지 않습니다."
                } else if !function.checks {
                    "의 시험에 확인하는 줄(assert…)이 없습니다."
                } else {
                    ""
                };
                reasons.entry(number).or_default().push(reason);
            }
        }
        for number in forbidden_ids(&project.card(unit)) {
            total += 1;
            match reasons.get(&number) {
                None => problems.push(format!("{rel}: 금지 사항 [{number}] 을 확인하는 시험이 없습니다. 시험 이름이 n{}_ 로 시작해야 합니다.", &number[1..])),
                Some(found) if !found.contains(&"") => problems.push(format!("{rel}: 금지 사항 [{number}] {}", found[0])),
                Some(_) => {}
            }
        }
    }
    counted(problems, format!("카드의 금지 사항 {total}개마다 실제로 도는 시험이 있습니다."))
}

fn allowed_std(path: &[String], app: bool) -> bool {
    let item = |list: &[(&str, &str)]| list.iter().any(|(module, name)| path.len() >= 2 && path[0] == *module && path[1] == *name);
    PURE_STD.contains(&path[0].as_str()) || item(&PURE_STD_ITEMS) || (app && item(&APP_STD_ITEMS))
}

pub fn doors(project: &Project) -> Gate {
    let mut problems = project.manifests.deps.clone();
    for dir in project.crates() {
        let app = dir == "app";
        let who = if app { "조립 파일이" } else { "블록이" };
        for file in project.sources(&dir, &["src", "tests"]) {
            let Some(tokens) = project.tokens(file) else {
                problems.push(format!("{file} 읽지 못했습니다. 글자가 깨졌거나 괄호의 짝이 맞지 않습니다."));
                continue;
            };
            // 다른 자리의 파일을 코드로 끌어오면 「자리 밖에 놓인 코드가 없다」는 카드 관문의 확인이 뜻을 잃는다. 그래서 통로와 시험에서도 막는다.
            let macros = scan::macro_calls(&tokens);
            let pulled = scan::attributes(&tokens).into_iter().filter(|(_, text)| text.starts_with("path=")).map(|(line, _)| (line, "#[path]"));
            for (line, how) in pulled.chain(macros.iter().filter(|(_, name)| name == "include").map(|(line, _)| (*line, "include!"))) {
                problems.push(format!("{file}:{line} 다른 자리의 파일을 코드로 끌어왔습니다({how}). 코드는 자기 폴더의 src/ 와 tests/ 에 둡니다."));
            }
            // 아래는 계산과 조립만 하는 자리(블록과 조립의 src/)에 건다. 통로와, tests/ 폴더의 시험은 바깥을 쓸 수 있다.
            if dir.starts_with("doors/") || !file.starts_with(&format!("{dir}/src/")) {
                continue;
            }
            for (line, path) in scan::std_paths(&tokens) {
                if path.is_empty() {
                    let how = "어디에 닿는지 알 수 있게 std::모듈::이름 까지 적고, 모듈·함수·변수에는 std·core·alloc 이라는 이름을 붙이지 않습니다.";
                    problems.push(format!("{file}:{line} {who} 표준 라이브러리를 경로 없이 이름으로만 썼습니다. {how}"));
                } else if !allowed_std(&path, app) {
                    problems.push(format!("{file}:{line} {who} 허용되지 않은 표준 라이브러리 std::{} 을 썼습니다. 바깥과 닿는 일은 doors/ 에 둡니다.", path.join("::")));
                }
            }
            for (line, name) in &macros {
                if OUTSIDE_MACROS.contains(&name.as_str()) || (!app && SCREEN_MACROS.contains(&name.as_str())) {
                    problems.push(format!("{file}:{line} {who} {name}! 를 썼습니다. 바깥과 닿는 일은 doors/ 에 둡니다."));
                }
            }
            for (line, word) in scan::words(&tokens, &["unsafe", "extern", "RandomState"]) {
                problems.push(format!("{file}:{line} {who} {word} 를 썼습니다. 컴파일러가 확인하지 못하는 코드, 다른 언어의 함수, 난수는 doors/ 에 둡니다."));
            }
            if app {
                for line in scan::decisions(&tokens) {
                    problems.push(format!("{file}:{line} 조립 파일에 판단(if·match·for·while·클로저·&&)이 있습니다. 판단은 블록에 둡니다."));
                }
            }
        }
    }
    counted(problems, "블록과 조립 파일이 바깥을 직접 건드리지 않고, 조립 파일에 판단이 없으며, 외부 부품은 승인된 것만 씁니다.".to_string())
}

/// 검사 하나를 가리키는 이름인가. 묶음 이름과 형태가 틀린 이름은 아니다.
fn single_lint(lint: &str) -> bool {
    let name = lint.strip_prefix("clippy::").unwrap_or(lint);
    !name.is_empty() && name.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_') && !LINT_GROUPS.contains(&lint)
}

pub fn patterns(project: &Project) -> Gate {
    let mut problems = project.manifests.config.clone();
    // 예외는 프로젝트 안의 파일 하나에 검사 이름 하나씩만 받는다.
    for (file, lints) in &project.exceptions {
        if !file.chars().all(|letter| letter.is_alphanumeric() || "._/-".contains(letter)) || !project.is_file(file) {
            problems.push(format!("pattern-exceptions 의 「{file}」: 프로젝트 안에 있는 파일 하나의 경로를 적습니다."));
        }
        for lint in lints.iter().filter(|lint| !single_lint(lint)) {
            problems.push(format!("pattern-exceptions 의 「{lint}」: 검사 이름 하나를 적습니다(예: clippy::too_many_arguments). 검사 묶음은 받지 않습니다."));
        }
    }
    // 빌드와 검사를 바꾸는 설정 파일. 맨 위의 .cargo 와 rust-toolchain 은 실행기(check)가 먼저 막지만, 다른 자리에 놓인 것도 받지 않는다.
    for (rel, _) in &project.entries {
        // 대소문자를 가리지 않는 파일 시스템에서는 Clippy.toml 도 읽힌다.
        let name = rel.rsplit('/').next().unwrap_or(rel).to_ascii_lowercase();
        if matches!(name.as_str(), ".cargo" | "clippy.toml" | ".clippy.toml" | "rust-toolchain") || (name == "rust-toolchain.toml" && rel != "rust-toolchain.toml") {
            problems.push(format!("{rel}: 빌드와 검사를 바꾸는 설정 파일은 프로젝트에 둘 수 없습니다."));
        }
    }
    for dir in project.crates() {
        for file in project.sources(&dir, &["src", "tests"]) {
            let Some(tokens) = project.tokens(file) else { continue };
            // 코드 안의 표시로는 검사를 끌 수 없다. 예외는 맨 위 Cargo.toml 의 pattern-exceptions 에 적힌 것만 받는다.
            let allows = scan::allows(&tokens);
            for (line, lint) in &allows {
                if !project.exceptions.get(file).is_some_and(|waived| waived.contains(lint)) {
                    let how = "꼭 필요한 예외는 맨 위 Cargo.toml 의 pattern-exceptions 에 적고 보고합니다.";
                    problems.push(format!("{file}:{line} 검사를 끄는 표시(allow·expect·warn)가 있습니다: {lint}. {how}"));
                }
            }
            if !allows.is_empty() && scan::opens_other_files(&tokens) {
                problems.push(format!("{file}: 검사를 끄는 표시가 있는 파일에는 다른 파일을 여는 「mod 이름;」 선언을 둘 수 없습니다. 표시가 그 파일들에까지 걸리기 때문입니다."));
            }
            for (line, text) in scan::attributes(&tokens) {
                if text.starts_with("cfg_attr(") || (text.starts_with("cfg(") && text != "cfg(test)") {
                    problems.push(format!("{file}:{line} 조건에 따라 코드를 빼는 표시는 #[cfg(test)] 만 씁니다. 다른 조건은 코드를 검사와 시험에서 빠지게 합니다."));
                }
            }
            // 시험할 때만 금지 사항을 지키는 코드를 막는다. 시험 모듈 밖의 #[cfg(test)] 와 cfg! 는 시험할 때와 실제로 돌 때의 코드를 다르게 만든다.
            for (line, on_module) in scan::test_only(&tokens) {
                if !on_module {
                    problems.push(format!("{file}:{line} #[cfg(test)] 는 시험 모듈(mod 이름) 바로 앞에만 붙입니다. 다른 자리에 붙이면 시험할 때와 실제로 돌 때의 코드가 달라집니다."));
                }
            }
            for (line, name) in scan::macro_calls(&tokens) {
                if name == "cfg" || name == "cfg_select" {
                    problems.push(format!("{file}:{line} {name}! 를 썼습니다. 시험할 때와 실제로 돌 때의 코드가 달라지므로 쓰지 않습니다."));
                }
            }
        }
    }
    let args = [&["clippy", "--workspace", "--all-targets", "--locked", "--message-format=json", "--"], &LINT_FLAGS[..]].concat();
    match project.run("cargo", &args, "", LINT_LIMIT) {
        Ok(done) if done.code == Some(0) => {}
        Ok(done) => {
            let found = diagnostics(&json_lines(&done.stdout));
            if found.is_empty() {
                // 도구가 안 도는 상태를 「점검 못 함」으로 두면 이 관문이 꺼진 채 굳는다. 그래서 실패로 본다.
                problems.push("패턴 검사 도구(clippy)를 실행하지 못했습니다.".to_string());
                problems.extend(last_lines(&done.stderr, 2));
            }
            problems.extend(found);
        }
        Err(Failure::Timeout) => problems.push("패턴 검사가 시간 안에 끝나지 않았습니다.".to_string()),
        Err(Failure::Spawn(error)) => problems.push(format!("빌드 도구(cargo)를 실행하지 못했습니다({error}).")),
    }
    if problems.is_empty() { pass("위험하거나 틀린 코드 패턴이 없습니다.") } else { fail(format!("{}건(코드에 붙은 설명은 도구가 낸 영어 원문입니다)", problems.len()), problems) }
}

pub fn secrets(project: &Project) -> Gate {
    let mut problems = Vec::new();
    for file in project.files() {
        let Ok(bytes) = fs::read(project.root.join(file)) else {
            problems.push(format!("{file}: 읽지 못했습니다."));
            continue;
        };
        // 글자 파일만 본다. 첫머리에 0 바이트가 있으면 글자 파일이 아닌 것으로 본다.
        if bytes.iter().take(8192).any(|&byte| byte == 0) {
            continue;
        }
        for (number, line) in bytes.split(|&byte| byte == b'\n').enumerate() {
            if let Some(label) = scan::secret_in(line) {
                problems.push(format!("{file}:{} {label}", number + 1));
            }
        }
    }
    if problems.is_empty() { pass("알려진 형태의 비밀 값은 발견되지 않았습니다.") } else { fail(format!("{}건. 비밀 값은 프로젝트 폴더 안에 두지 않습니다.", problems.len()), problems) }
}

/// 주인에게 알리고 묻는 장치(훅, 잠금·승인 규칙)가 설정 파일에 그대로 적혀 있는지, 고정한 Rust 버전으로 도는지 본다.
/// 장치가 빠져도 다른 관문은 통과로 나오기 때문이다.
pub fn devices(project: &Project) -> Gate {
    // 형식이 틀린 값이 하나라도 섞이면 Claude Code 가 설정 파일을 통째로 읽지 않는다. 그래서 틀이 정한 내용만 있어야 한다.
    // 훅도 명령만 같아서는 모자라다. async 를 더하거나 timeout 을 줄이면 막지 못하므로 항목 전체가 같아야 한다.
    let expected = json!({
        "permissions": {"deny": DENY_RULES, "ask": ASK_RULES},
        "hooks": {"Stop": [{"hooks": [{"type": "command", "command": HOOK_COMMAND, "timeout": 300}]}]},
    });
    let mut problems = Vec::new();
    for name in ["settings.json", "settings.local.json"] {
        let settings = match fs::read_to_string(project.root.join(".claude").join(name)) {
            Ok(text) => serde_json::from_str::<Value>(&text).ok(),
            Err(_) => Some(json!({})),
        };
        let Some(settings) = settings else {
            problems.push(format!(".claude/{name}: JSON 형식이 틀렸습니다. Claude Code 가 이 파일을 읽지 못합니다."));
            continue;
        };
        if !matches!(settings.get("disableAllHooks"), None | Some(Value::Null | Value::Bool(false))) {
            problems.push(format!(".claude/{name}: 훅을 전부 끄는 설정(disableAllHooks)이 있습니다."));
        } else if name == "settings.json" && settings != expected {
            let rules = format!("답을 끝낼 때마다 도는 점검 훅, 잠금 규칙 {}개, 승인 규칙 {}개만 적혀 있어야 합니다.", DENY_RULES.len(), ASK_RULES.len());
            let how = "이 파일은 잠겨 있으므로 상위 폴더에서 연 세션에서 되돌리고, 다른 설정은 .claude/settings.local.json 에 적습니다.";
            problems.push(format!(".claude/settings.json: 틀이 정한 내용과 다릅니다. {rules} {how}"));
        }
    }
    for (name, expected) in harness_hooks() {
        let written = fs::read_to_string(project.root.join(name)).ok().and_then(|text| serde_json::from_str::<Value>(&text).ok());
        if written != Some(expected) {
            problems.push(format!("{name}: 틀이 정한 내용과 다릅니다. 이 파일은 잠겨 있으므로 상위 폴더에서 연 세션에서 되돌립니다."));
        }
    }
    let codex = fs::read_to_string(project.root.join(".codex/config.toml")).ok().and_then(|text| text.parse::<toml::Table>().ok());
    if codex.as_ref().and_then(|table| table.get("features")).is_some_and(|features| ["hooks", "codex_hooks"].iter().any(|key| features.get(key) == Some(&toml::Value::Boolean(false)))) {
        problems.push(".codex/config.toml: 훅을 끄는 설정(features.hooks = false)이 있습니다.".to_string());
    }
    // rust-toolchain.toml 이 고정한 버전으로 실제로 도는지 본다. 다른 버전이 돌면 패턴 검사의 내용이 달라진다.
    let pinned = fs::read_to_string(project.root.join("rust-toolchain.toml")).ok().and_then(|text| text.parse::<toml::Table>().ok());
    let pinned = pinned.as_ref().and_then(|table| table.get("toolchain")?.get("channel")?.as_str());
    let running = project.run("rustc", &["--version"], "", TEST_LIMIT).ok().map(|done| done.stdout);
    let running = running.as_deref().and_then(|text| text.split_whitespace().nth(1));
    match (pinned, running) {
        (None, _) => problems.push("rust-toolchain.toml: Rust 버전이 고정돼 있지 않습니다.".to_string()),
        (Some(pinned), running) if running != Some(pinned) => {
            let how = "rustup 으로 깐 Rust 가 PATH 의 앞쪽에 있어야 합니다.";
            problems.push(format!("rust-toolchain.toml: 고정한 Rust 버전({pinned})과 실제로 도는 버전({})이 다릅니다. {how}", running.unwrap_or("알 수 없음")));
        }
        _ => {}
    }
    let good = format!(
        "답을 끝낼 때마다 도는 점검, 잠금 규칙 {}개, 승인 규칙 {}개와 Codex·Antigravity 훅이 설정 파일에 그대로 적혀 있고, 고정한 Rust 버전으로 돕니다. 하네스가 훅을 실제로 켰는지는 이 관문이 보지 못합니다.",
        DENY_RULES.len(),
        ASK_RULES.len()
    );
    counted(problems, good)
}

pub fn dependencies(project: &Project) -> Gate {
    if project.offline {
        return skip("점검하지 못했습니다: 인터넷을 쓰지 않도록 실행했습니다(--offline).");
    }
    // 훅은 PATH 가 짧은 환경에서 돌 수 있어서 흔한 설치 위치를 직접 본다.
    let home = std::env::var("HOME").unwrap_or_default();
    let programs = ["cargo-audit".to_string(), "/opt/homebrew/bin/cargo-audit".to_string(), "/usr/local/bin/cargo-audit".to_string(), format!("{home}/.cargo/bin/cargo-audit")];
    let mut found = Vec::new();
    // 프로젝트의 부품과 점검 명령의 부품을 둘 다 본다. 취약점 목록은 처음 한 번만 새로 받는다.
    for (index, lock) in ["Cargo.lock", "tools/check/Cargo.lock"].into_iter().filter(|lock| project.is_file(lock)).enumerate() {
        let args = [&["audit", "--json", "--file", lock][..], if index > 0 { &["--no-fetch"] } else { &[] }].concat();
        let outcome = programs.iter().find_map(|program| match project.run(program, &args, "", AUDIT_LIMIT) {
            Err(Failure::Spawn(_)) => None,
            other => Some(other),
        });
        let done = match outcome {
            None => return skip("점검하지 못했습니다: cargo-audit 를 찾지 못했습니다. 에이전트에게 「cargo-audit 를 깔아 줘」라고 말하면 됩니다."),
            Some(Err(_)) => return skip("점검하지 못했습니다: 30초 안에 끝나지 않았습니다. 인터넷이 느리거나 끊겼을 수 있습니다."),
            Some(Ok(done)) => done,
        };
        // 확실히 읽힌 경우만 통과·실패로 보고 나머지는 「점검 못 함」으로 둔다.
        let report: Value = serde_json::from_str(&done.stdout).unwrap_or(Value::Null);
        let Some(list) = report["vulnerabilities"]["list"].as_array() else {
            let reason = if done.stderr.contains("fetch advisory database") { Some("인터넷 연결이 필요합니다.".to_string()) } else { last_lines(&done.stderr, 1).pop() };
            return skip(format!("점검하지 못했습니다: {}", reason.unwrap_or("알 수 없는 오류".to_string())));
        };
        for item in list {
            let text = |value: &Value| value.as_str().unwrap_or("?").to_string();
            found.push(format!("{} {}: 알려진 취약점 {}", text(&item["package"]["name"]), text(&item["package"]["version"]), text(&item["advisory"]["id"])));
        }
    }
    if found.is_empty() { pass("가져다 쓰는 외부 부품에 알려진 취약점이 없습니다.") } else { fail("취약점이 알려진 외부 부품이 있습니다.", found) }
}
