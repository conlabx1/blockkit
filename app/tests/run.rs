//! 조립된 전체를 실제 실행 파일로 돌려 본다. 바깥(네트워크 등)에는 닿지 않는다.

use std::io::Write;
use std::process::{Command, Stdio};

/// `typed` 는 표준 입력으로 넣는 글자다. 다 넣고 나면 입력을 닫는다.
/// 이름을 인자로 줄 때는 `typed` 를 비워 둔다. 입력을 읽지 않고 끝나는 프로그램에 글자를 넣으면 시험이 가끔 실패한다.
fn run_app(args: &[&str], typed: &str) -> (Option<i32>, String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_app"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("실행 파일을 돌리지 못했다");
    child.stdin.take().expect("입력을 넣을 자리가 없다").write_all(typed.as_bytes()).expect("입력을 넣지 못했다");
    let done = child.wait_with_output().expect("실행 파일이 끝나지 않았다");
    let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
    (done.status.code(), text(&done.stdout), text(&done.stderr))
}

#[test]
fn 이름을_주면_인사말을_내보낸다() {
    assert_eq!(run_app(&["하늘"], ""), (Some(0), "하늘님, 안녕하세요.\n".to_string(), String::new()));
}

#[test]
fn 이름을_주지_않으면_입력에서_한_줄을_읽어_인사한다() {
    assert_eq!(run_app(&[], "바다\n"), (Some(0), "바다님, 안녕하세요.\n".to_string(), String::new()));
}

#[test]
fn 너무_긴_줄을_넣으면_오류를_알리고_실패로_끝난다() {
    let (code, out, err) = run_app(&[], &("a".repeat(1001) + "\n"));
    assert_eq!((code, out.as_str()), (Some(1), ""));
    assert!(err.contains("입력이 너무 깁니다"), "{err}");
}

#[test]
fn 이름도_입력도_없으면_오류를_알리고_실패로_끝난다() {
    let (code, out, err) = run_app(&[], "");
    assert_eq!((code, out.as_str()), (Some(1), ""));
    assert!(err.contains("입력이 없습니다"), "{err}");
}
