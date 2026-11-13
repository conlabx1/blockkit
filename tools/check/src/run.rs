//! 다른 프로그램을 실행한다. 환경 변수는 실행에 필요한 것만 넘기고, 정해진 시각이 지나면 멈춘다.

use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// 시험은 방금 쓰인 코드를 그대로 실행한다. 주인의 환경 변수에 든 토큰이 그 코드로 넘어가지 않게, 실행에 필요한 것만 넘긴다.
/// 컴파일러와 검사 도구의 동작을 바꾸는 환경 변수(RUSTFLAGS 등)도 같이 걸러진다.
const PASSED_ON: [&str; 4] = ["PATH", "HOME", "TMPDIR", "LANG"];

pub struct Done {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

pub enum Failure {
    /// 정해진 시각까지 끝나지 않았다.
    Timeout,
    /// 프로그램을 띄우지 못했다.
    Spawn(String),
}

pub fn run(program: &str, args: &[&str], dir: &Path, until: Instant) -> Result<Done, Failure> {
    let mut command = Command::new(program);
    command.args(args).current_dir(dir).env_clear().stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    command.envs(PASSED_ON.iter().filter_map(|name| Some((name, std::env::var_os(name)?))));
    // 띄운 프로그램이 또 띄운 프로그램(컴파일러 등)까지 한꺼번에 멈출 수 있게 묶음을 따로 만든다.
    command.process_group(0);
    let mut child = command.spawn().map_err(|error| Failure::Spawn(error.to_string()))?;
    let (out, err) = (drain(child.stdout.take()), drain(child.stderr.take()));
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() < until => thread::sleep(Duration::from_millis(2)),
            _ => break None,
        }
    };
    // 남아서 출력 통로를 붙잡고 있는 프로그램이 있으면 아래의 읽기가 끝나지 않는다. 그래서 묶음을 통째로 멈춘다.
    let _ = Command::new("/bin/kill").args(["-KILL", &format!("-{}", child.id())]).stderr(Stdio::null()).status();
    let _ = child.wait();
    // 시험이 화면에 글자가 아닌 바이트를 써도 죽지 않게, 읽을 수 없는 바이트는 바꿔서 읽는다.
    let text = |reader: thread::JoinHandle<Vec<u8>>| String::from_utf8_lossy(&reader.join().unwrap_or_default()).into_owned();
    match status {
        Some(status) => Ok(Done { code: status.code(), stdout: text(out), stderr: text(err) }),
        None => Err(Failure::Timeout),
    }
}

fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(mut pipe) = pipe {
            let _ = pipe.read_to_end(&mut bytes);
        }
        bytes
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn soon(millis: u64) -> Instant {
        Instant::now() + Duration::from_millis(millis)
    }

    #[test]
    fn 출력과_종료_코드를_돌려준다() {
        let done = run("/bin/sh", &["-c", "printf 'a\\377b'; echo 오류 >&2; exit 3"], Path::new("/"), soon(5000)).ok().unwrap();
        assert_eq!((done.code, done.stdout.as_str(), done.stderr.as_str()), (Some(3), "a\u{fffd}b", "오류\n"));
    }

    #[test]
    fn 환경_변수는_정해진_것만_넘긴다() {
        // 시험 도중에 환경 변수를 바꾸지 않으려고, cargo 가 시험에 늘 넘겨 주는 변수가 걸러지는지를 본다.
        assert!(std::env::var_os("CARGO_MANIFEST_DIR").is_some());
        let done = run("/usr/bin/env", &[], Path::new("/"), soon(5000)).ok().unwrap();
        let names: Vec<&str> = done.stdout.lines().filter_map(|line| line.split('=').next()).collect();
        assert!(names.contains(&"PATH"), "{names:?}");
        assert!(names.iter().all(|name| PASSED_ON.contains(name)), "{names:?}");
    }

    #[test]
    fn 시간을_넘기면_띄운_프로그램까지_멈춘다() {
        let started = Instant::now();
        // 안쪽 sleep 이 살아남으면 출력 통로가 닫히지 않아서 30초를 기다리게 된다.
        let outcome = run("/bin/sh", &["-c", "sleep 30 & sleep 30"], Path::new("/"), soon(300));
        assert!(matches!(outcome, Err(Failure::Timeout)));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn 없는_프로그램은_띄우지_못한_것으로_알린다() {
        assert!(matches!(run("/없는/프로그램", &[], Path::new("/"), soon(1000)), Err(Failure::Spawn(_))));
    }
}
