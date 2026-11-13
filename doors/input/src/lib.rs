//! 프로그램의 표준 입력에서 한 줄을 읽어 온다. 약속은 CARD.md 에 있다.

use std::io::BufRead;

/// 받는 줄의 최대 길이(바이트). 줄 끝의 줄바꿈은 세지 않는다.
const LIMIT: usize = 1000;

/// 표준 입력에서 한 줄을 읽는다. 읽지 못했거나 받을 수 없는 줄이면 그 이유를 돌려준다.
pub fn line() -> Result<String, String> {
    line_from(std::io::stdin().lock())
}

fn line_from(reader: impl BufRead) -> Result<String, String> {
    let mut bytes = Vec::new();
    // 한도에 줄 끝(\r\n) 두 바이트를 더한 만큼만 읽는다. 끝없이 긴 줄이 와도 그 이상은 메모리에 올리지 않는다.
    reader.take(LIMIT as u64 + 2).read_until(b'\n', &mut bytes).map_err(|error| format!("입력을 읽지 못했습니다: {error}"))?;
    if bytes.is_empty() {
        return Err("입력이 없습니다.".to_string());
    }
    let line = bytes.strip_suffix(b"\n").unwrap_or(&bytes);
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    if line.len() > LIMIT {
        return Err(format!("입력이 너무 깁니다. 한 줄은 {LIMIT}바이트까지 받습니다."));
    }
    String::from_utf8(line.to_vec()).map_err(|_| "입력이 글자(UTF-8)가 아닙니다.".to_string())
}

#[cfg(test)]
mod tests {
    use super::{LIMIT, line_from};

    #[test]
    fn 첫_줄을_읽어_줄바꿈을_떼고_돌려준다() {
        for typed in ["하늘\n", "하늘\r\n", "하늘", "하늘\n바다\n"] {
            assert_eq!(line_from(typed.as_bytes()).as_deref(), Ok("하늘"));
        }
        // 빈 줄은 읽은 것이 있는 경우다. 빈 글자를 받을지는 받는 블록이 정한다.
        assert_eq!(line_from(&b"\n"[..]).as_deref(), Ok(""));
    }

    #[test]
    fn n1_읽을_것이_없으면_빈_글자를_돌려주지_않는다() {
        assert!(line_from(&b""[..]).is_err());
    }

    #[test]
    fn n2_한도를_넘는_줄은_받지_않는다() {
        let longest = "가".repeat(LIMIT / 3) + "a";
        assert_eq!((longest.len(), line_from(format!("{longest}\r\n").as_bytes())), (LIMIT, Ok(longest.clone())));
        for long in [format!("{longest}a\n"), format!("{longest}\ra\n")] {
            assert!(line_from(long.as_bytes()).is_err(), "{long:?}");
        }
        // 긴 입력은 한도 근처까지만 읽고 나머지는 건드리지 않는다.
        let long = vec![b'a'; LIMIT * 5];
        let mut rest = &long[..];
        assert!(line_from(&mut rest).is_err());
        assert!(rest.len() >= LIMIT * 5 - LIMIT - 2, "{}", rest.len());
    }
}
