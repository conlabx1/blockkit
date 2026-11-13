//! 이름을 받아 인사말 한 줄을 만든다. 약속은 CARD.md 에 있다.

/// 인사말을 만든다. 받을 수 없는 이름이면 그 이유를 돌려준다.
pub fn greet(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("이름이 비어 있습니다.".to_string());
    }
    if name.contains(['\n', '\r']) {
        return Err("이름에 줄바꿈이 들어 있습니다.".to_string());
    }
    Ok(format!("{name}님, 안녕하세요."))
}

/// 이름이 비어 있으면 `ask` 로 한 번 물어서 받은 이름으로 인사말을 만든다.
pub fn greet_or_ask(name: &str, ask: impl FnOnce() -> Result<String, String>) -> Result<String, String> {
    if name.trim().is_empty() {
        return greet(&ask()?);
    }
    greet(name)
}

#[cfg(test)]
mod tests {
    use super::{greet, greet_or_ask};

    #[test]
    fn 이름을_넣으면_인사말이_나온다() {
        assert_eq!(greet("하늘").as_deref(), Ok("하늘님, 안녕하세요."));
    }

    #[test]
    fn 앞뒤_공백은_떼고_인사한다() {
        assert_eq!(greet("  하늘 \n").as_deref(), Ok("하늘님, 안녕하세요."));
    }

    #[test]
    fn 이름을_받지_못했으면_물어서_받은_이름으로_인사한다() {
        assert_eq!(greet_or_ask(" ", || Ok("하늘".to_string())).as_deref(), Ok("하늘님, 안녕하세요."));
    }

    #[test]
    fn 묻다가_난_오류는_그대로_알린다() {
        assert_eq!(greet_or_ask("", || Err("입력이 없습니다.".to_string())), Err("입력이 없습니다.".to_string()));
    }

    #[test]
    fn n1_빈_이름으로는_인사말을_만들지_않는다() {
        for name in ["", "   "] {
            assert!(greet(name).is_err());
            assert!(greet_or_ask("", || Ok(name.to_string())).is_err());
        }
    }

    #[test]
    fn n2_줄바꿈이_든_이름은_받지_않는다() {
        for name in ["하\n늘", "하\r늘"] {
            assert!(greet(name).is_err(), "{name:?}");
            assert!(greet_or_ask("", || Ok(name.to_string())).is_err(), "{name:?}");
        }
    }

    #[test]
    fn n3_이름을_이미_받았으면_다시_묻지_않는다() {
        assert_eq!(greet_or_ask("하늘", || unreachable!("이름이 있는데 다시 물었다")).as_deref(), Ok("하늘님, 안녕하세요."));
    }
}
