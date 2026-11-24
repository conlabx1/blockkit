//! 소스 글자를 훑어서 사실만 뽑는다. 파일을 읽거나 프로그램을 실행하지 않고, 무엇이 위반인지도 정하지 않는다.

use proc_macro2::{Delimiter, Spacing, TokenStream, TokenTree};

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Ident(String),
    /// (글자, 바로 뒤에 다른 기호가 붙어 있는가)
    Punct(char, bool),
    Open(char),
    Close(char),
    Literal,
}

#[derive(Debug, Clone)]
pub struct Flat {
    pub line: usize,
    pub tok: Tok,
}

/// 소스를 토큰으로 나누고 괄호 안쪽까지 한 줄로 편다.
/// 매크로 호출 안쪽도 같은 토큰이라서, 매크로 안에 적은 경로와 이름도 함께 보인다. 주석은 토큰이 아니고, 글자 값은 내용 없이 자리만 남는다.
pub fn flatten(source: &str) -> Result<Vec<Flat>, String> {
    let stream: TokenStream = source.parse().map_err(|error| format!("{error}"))?;
    let mut out = Vec::new();
    push(stream, &mut out);
    Ok(out)
}

fn push(stream: TokenStream, out: &mut Vec<Flat>) {
    for tree in stream {
        let line = tree.span().start().line;
        match tree {
            TokenTree::Group(group) => {
                let (open, close) = match group.delimiter() {
                    Delimiter::Parenthesis => ('(', ')'),
                    Delimiter::Brace => ('{', '}'),
                    Delimiter::Bracket => ('[', ']'),
                    Delimiter::None => (' ', ' '),
                };
                out.push(Flat { line, tok: Tok::Open(open) });
                push(group.stream(), out);
                out.push(Flat { line: group.span_close().start().line, tok: Tok::Close(close) });
            }
            // r#std 처럼 적어도 같은 이름으로 본다.
            TokenTree::Ident(ident) => out.push(Flat { line, tok: Tok::Ident(ident.to_string().trim_start_matches("r#").to_string()) }),
            TokenTree::Punct(punct) => out.push(Flat { line, tok: Tok::Punct(punct.as_char(), punct.spacing() == Spacing::Joint) }),
            TokenTree::Literal(_) => out.push(Flat { line, tok: Tok::Literal }),
        }
    }
}

fn ident(tokens: &[Flat], at: usize) -> Option<&str> {
    match tokens.get(at).map(|flat| &flat.tok) {
        Some(Tok::Ident(name)) => Some(name),
        _ => None,
    }
}

fn punct(tokens: &[Flat], at: usize, wanted: char) -> bool {
    matches!(tokens.get(at).map(|flat| &flat.tok), Some(Tok::Punct(found, _)) if *found == wanted)
}

fn opens(tokens: &[Flat], at: usize, wanted: char) -> bool {
    matches!(tokens.get(at).map(|flat| &flat.tok), Some(Tok::Open(found)) if *found == wanted)
}

fn colons(tokens: &[Flat], at: usize) -> bool {
    matches!(tokens.get(at).map(|flat| &flat.tok), Some(Tok::Punct(':', true))) && punct(tokens, at + 1, ':')
}

/// 여는 괄호의 자리를 받아서 짝이 되는 닫는 괄호의 자리를 돌려준다.
fn group_end(tokens: &[Flat], open: usize) -> usize {
    let mut depth = 0usize;
    for (index, flat) in tokens.iter().enumerate().skip(open) {
        match flat.tok {
            Tok::Open(_) => depth += 1,
            Tok::Close(_) => {
                depth -= 1;
                if depth == 0 {
                    return index;
                }
            }
            _ => {}
        }
    }
    tokens.len()
}

/// 표준 라이브러리로 들어가는 경로를 (줄, std 뒤의 조각들) 로 돌려준다. `use std::{fs, io}` 는 조각마다 하나씩 나온다.
/// 조각이 비어 있으면 뒤에 경로 없이 이름만 쓴 것이다(`use std as s`, 매크로에 `std` 만 넘기기). 어디로 가는지 알 수 없는 모양이다.
pub fn std_paths(tokens: &[Flat]) -> Vec<(usize, Vec<String>)> {
    let mut found = Vec::new();
    for (index, flat) in tokens.iter().enumerate() {
        if !matches!(ident(tokens, index), Some("std" | "core" | "alloc")) {
            continue;
        }
        // `self.core` 같은 필드나 메서드 이름은 경로가 아니다. 범위(`0..std::…`)의 둘째 점 뒤에 온 것은 경로다.
        if index > 0 && punct(tokens, index - 1, '.') && !(index > 1 && matches!(tokens[index - 2].tok, Tok::Punct('.', true))) {
            continue;
        }
        if colons(tokens, index + 1) {
            let mut paths = Vec::new();
            tree(tokens, index + 3, Vec::new(), &mut paths);
            found.extend(paths.into_iter().map(|path| (flat.line, path)));
        } else {
            found.push((flat.line, Vec::new()));
        }
    }
    found
}

fn tree(tokens: &[Flat], at: usize, mut path: Vec<String>, out: &mut Vec<Vec<String>>) {
    match tokens.get(at).map(|flat| &flat.tok) {
        Some(Tok::Ident(name)) => {
            path.push(name.clone());
            if colons(tokens, at + 1) {
                tree(tokens, at + 3, path, out);
            } else {
                out.push(path);
            }
        }
        Some(Tok::Open('{')) => {
            let (mut depth, mut start) = (0usize, true);
            for index in at + 1..group_end(tokens, at) {
                let at_item = start && depth == 0;
                match tokens[index].tok {
                    Tok::Open(_) => depth += 1,
                    Tok::Close(_) => depth -= 1,
                    Tok::Punct(',', _) if depth == 0 => {
                        start = true;
                        continue;
                    }
                    _ => {}
                }
                if at_item {
                    tree(tokens, index, path.clone(), out);
                    start = false;
                }
            }
        }
        Some(Tok::Punct('*', _)) => {
            path.push("*".to_string());
            out.push(path);
        }
        _ => {
            path.push("?".to_string());
            out.push(path);
        }
    }
}

/// 매크로 호출을 (줄, 이름) 으로 돌려준다.
pub fn macro_calls(tokens: &[Flat]) -> Vec<(usize, String)> {
    let mut found = Vec::new();
    for (index, flat) in tokens.iter().enumerate() {
        if let Some(name) = ident(tokens, index)
            && punct(tokens, index + 1, '!')
            && matches!(tokens.get(index + 2).map(|next| &next.tok), Some(Tok::Open(_)))
        {
            found.push((flat.line, name.to_string()));
        }
    }
    found
}

/// 주어진 낱말이 이름으로 쓰인 자리를 (줄, 낱말) 로 돌려준다.
pub fn words(tokens: &[Flat], wanted: &[&str]) -> Vec<(usize, String)> {
    let mut found = Vec::new();
    for (index, flat) in tokens.iter().enumerate() {
        if let Some(name) = ident(tokens, index)
            && wanted.contains(&name)
        {
            found.push((flat.line, name.to_string()));
        }
    }
    found
}

/// 판단이 든 줄: 분기와 반복의 낱말, 클로저와 「또는」의 `|`, 「그리고」의 `&&`.
// shortcut: 판단을 표에서 고르기나 함수 넘기기로 바꿔 쓴 것까지는 보지 못한다.
pub fn decisions(tokens: &[Flat]) -> Vec<usize> {
    let mut lines: Vec<usize> = Vec::new();
    // `impl 약속 for 자료형` 의 for 는 반복이 아니다. impl 부터 몸통이 열리거나 문장이 끝날 때까지를 머리로 본다.
    let mut header = false;
    for (index, flat) in tokens.iter().enumerate() {
        let hit = match &flat.tok {
            Tok::Ident(word) if word == "impl" => {
                header = true;
                false
            }
            Tok::Open('{') | Tok::Punct(';', _) => {
                header = false;
                false
            }
            Tok::Ident(word) if word == "for" => !header,
            Tok::Ident(word) => matches!(word.as_str(), "if" | "match" | "while" | "loop" | "else"),
            Tok::Punct('|', _) => true,
            Tok::Punct('&', true) => punct(tokens, index + 1, '&'),
            _ => false,
        };
        if hit && lines.last() != Some(&flat.line) {
            lines.push(flat.line);
        }
    }
    lines
}

/// 다른 파일을 여는 선언(`mod 이름;`)이 있는가.
pub fn opens_other_files(tokens: &[Flat]) -> bool {
    (0..tokens.len()).any(|index| ident(tokens, index) == Some("mod") && ident(tokens, index + 1).is_some() && punct(tokens, index + 2, ';'))
}

/// 속성(`#[…]`, `#![…]`) 하나하나의 (# 의 자리, 여는 괄호의 자리, 닫는 괄호의 자리).
fn attribute_spans(tokens: &[Flat]) -> Vec<(usize, usize, usize)> {
    let mut found = Vec::new();
    for index in 0..tokens.len() {
        if punct(tokens, index, '#') {
            let open = if punct(tokens, index + 1, '!') { index + 2 } else { index + 1 };
            if opens(tokens, open, '[') {
                found.push((index, open, group_end(tokens, open)));
            }
        }
    }
    found
}

/// 토큰을 빈칸 없이 이어 붙인 글자. `#[cfg(test)]` 의 안쪽은 `cfg(test)` 가 된다. 글자 값은 `_` 로 적는다.
fn render(tokens: &[Flat]) -> String {
    let mut text = String::new();
    for flat in tokens {
        match &flat.tok {
            Tok::Ident(name) => {
                if text.ends_with(|last: char| last.is_alphanumeric() || last == '_') {
                    text.push(' ');
                }
                text.push_str(name);
            }
            Tok::Punct(mark, _) | Tok::Open(mark) | Tok::Close(mark) => text.push(*mark),
            Tok::Literal => text.push('_'),
        }
    }
    text
}

/// 속성을 (줄, 안쪽 글자) 로 돌려준다. 문서 주석도 속성(`doc=_`)으로 나온다.
pub fn attributes(tokens: &[Flat]) -> Vec<(usize, String)> {
    attribute_spans(tokens).into_iter().map(|(hash, open, end)| (tokens[hash].line, render(&tokens[open + 1..end]))).collect()
}

/// `#[cfg(test)]` 가 붙은 자리마다 (줄, 바로 뒤에 오는 것이 모듈 선언인가) 를 돌려준다.
pub fn test_only(tokens: &[Flat]) -> Vec<(usize, bool)> {
    let spans = attribute_spans(tokens);
    let mut found = Vec::new();
    for (at, &(hash, open, end)) in spans.iter().enumerate() {
        if render(&tokens[open + 1..end]) != "cfg(test)" {
            continue;
        }
        // 바로 이어지는 다른 속성(문서 주석 포함)과 pub 은 건너뛴다.
        let mut next = end + 1;
        for span in &spans[at + 1..] {
            if span.0 != next {
                break;
            }
            next = span.2 + 1;
        }
        if ident(tokens, next) == Some("pub") {
            next = if opens(tokens, next + 1, '(') { group_end(tokens, next + 1) + 1 } else { next + 1 };
        }
        // 파일 맨 위에 적는 `#![cfg(test)]` 는 뒤에 무엇이 오든 모듈 앞이 아니다.
        found.push((tokens[hash].line, open == hash + 1 && ident(tokens, next) == Some("mod")));
    }
    found
}

/// 검사를 끄거나 낮추는 표시(`allow`, `expect`, `warn`)가 가리키는 검사 이름을 (줄, 이름) 으로 돌려준다.
/// `warn` 은 명령줄에서 실패로 올려 둔 검사를 경고로 되돌린다.
pub fn allows(tokens: &[Flat]) -> Vec<(usize, String)> {
    let mut found = Vec::new();
    for (_, open, end) in attribute_spans(tokens) {
        for index in open..end {
            if !matches!(ident(tokens, index), Some("allow" | "expect" | "warn")) || !opens(tokens, index + 1, '(') {
                continue;
            }
            // 쉼표로 나뉜 항목마다 이름 하나다. `reason = "…"` 은 검사 이름이 아니다.
            let (mut name, mut skip) = (String::new(), false);
            for inner in index + 2..=group_end(tokens, index + 1) {
                match &tokens[inner].tok {
                    Tok::Ident(part) => name.push_str(part),
                    Tok::Punct(':', _) => name.push(':'),
                    Tok::Punct('=', _) => skip = true,
                    Tok::Punct(',', _) | Tok::Close(_) => {
                        if !name.is_empty() && !skip {
                            found.push((tokens[index].line, name.clone()));
                        }
                        name.clear();
                        skip = false;
                    }
                    _ => {}
                }
            }
        }
    }
    found
}

#[derive(Debug, PartialEq)]
pub struct Function {
    pub name: String,
    /// `#[test]` 가 붙어 있는가.
    pub test: bool,
    /// 확인하는 줄이 있는가: assert 계열 매크로, panic!, unwrap·expect 호출, #[should_panic].
    pub checks: bool,
}

/// 소스에 적힌 함수를 돌려준다.
pub fn functions(tokens: &[Flat]) -> Vec<Function> {
    let spans = attribute_spans(tokens);
    let (mut found, mut pending) = (Vec::new(), Vec::<String>::new());
    let (mut index, mut next) = (0, 0);
    while index < tokens.len() {
        // 함수가 나올 때까지 속성을 모아 둔다. 시험 표시는 함수에만 붙으므로, 다른 것에 붙은 속성이 섞여도 결과는 같다.
        while spans.get(next).is_some_and(|span| span.0 < index) {
            next += 1;
        }
        if let Some(&(hash, open, end)) = spans.get(next)
            && hash == index
        {
            pending.push(render(&tokens[open + 1..end]));
            index = end + 1;
            continue;
        }
        match &tokens[index].tok {
            Tok::Ident(word) if word == "fn" => {
                if let Some(name) = ident(tokens, index + 1) {
                    let asserts = body(tokens, index + 2).is_some_and(|(open, end)| checks_in(tokens, open, end));
                    found.push(Function {
                        name: name.to_string(),
                        // `tokio::test(flavor = "…")` 처럼 인자가 붙어도 시험 표시다.
                        test: pending.iter().any(|text| matches!(text.split('(').next(), Some(head) if head == "test" || head.ends_with("::test"))),
                        // 멈춰야 통과하는 시험은 멈추는 것 자체가 확인이다.
                        checks: asserts || pending.iter().any(|text| text.starts_with("should_panic")),
                    });
                }
                pending.clear();
            }
            _ => {}
        }
        index += 1;
    }
    found
}

/// 함수 이름 뒤에서 처음 나오는 중괄호 묶음이 몸통이다. 인자 목록 같은 다른 묶음은 건너뛴다.
fn body(tokens: &[Flat], mut at: usize) -> Option<(usize, usize)> {
    loop {
        match tokens.get(at).map(|flat| &flat.tok) {
            Some(Tok::Open('{')) => return Some((at, group_end(tokens, at))),
            Some(Tok::Open(_)) => at = group_end(tokens, at) + 1,
            Some(Tok::Punct(';', _)) | None => return None,
            Some(_) => at += 1,
        }
    }
}

fn checks_in(tokens: &[Flat], open: usize, end: usize) -> bool {
    let mut at = open + 1;
    while at < end {
        if ident(tokens, at) == Some("fn")
            && ident(tokens, at + 1).is_some()
            && let Some((_, inner)) = body(tokens, at + 2)
        {
            at = inner + 1;
        } else if checks_at(tokens, at) {
            return true;
        } else {
            at += 1;
        }
    }
    false
}

fn checks_at(tokens: &[Flat], at: usize) -> bool {
    match ident(tokens, at) {
        // debug_assert 계열은 빌드 설정에 따라 사라질 수 있어서 확인으로 치지 않는다.
        Some(word) if punct(tokens, at + 1, '!') => word.starts_with("assert") || matches!(word, "panic" | "unreachable"),
        Some("unwrap" | "expect" | "unwrap_err" | "expect_err") => at > 0 && punct(tokens, at - 1, '.') && opens(tokens, at + 1, '('),
        _ => false,
    }
}

/// 금지 사항 번호가 붙은 시험 이름(`n1_…`)에서 번호(`N1`)를 꺼낸다.
pub fn forbidden_number(name: &str) -> Option<String> {
    let digits: String = name.strip_prefix('n')?.chars().take_while(char::is_ascii_digit).collect();
    (!digits.is_empty() && name[1 + digits.len()..].starts_with('_')).then(|| format!("N{digits}"))
}

fn word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// `prefix` 뒤로 `class` 에 드는 글자가 `min` 개 이상 이어지는 자리가 있는가. `exact` 면 정확히 `min` 개에서 낱말이 끝나야 한다.
fn token_after(line: &[u8], prefix: &[u8], class: fn(u8) -> bool, min: usize, exact: bool) -> bool {
    // 자리마다 `min` 개까지만 본다. 끝까지 세면 같은 글자가 길게 이어진 줄에서 걸리는 시간이 줄 길이의 제곱으로 늘어난다.
    line.windows(prefix.len()).enumerate().any(|(start, window)| {
        let rest = &line[start + prefix.len()..];
        window == prefix
            && (start == 0 || !word_byte(line[start - 1]))
            && rest.len() >= min
            && rest[..min].iter().all(|&byte| class(byte))
            && (!exact || rest.get(min).is_none_or(|&byte| !word_byte(byte)))
    })
}

// shortcut: 알려진 형태 몇 가지만 본다, 저장소를 남과 공유하게 되면 gitleaks 로 바꾼다.
/// 줄에 알려진 형태의 비밀 값이 있으면 그 종류를 돌려준다.
pub fn secret_in(line: &[u8]) -> Option<&'static str> {
    let upper_digit = |byte: u8| byte.is_ascii_uppercase() || byte.is_ascii_digit();
    let alnum = |byte: u8| byte.is_ascii_alphanumeric();
    let dashed = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'-';
    let url_safe = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_';
    if private_key(line) {
        Some("개인 키")
    } else if token_after(line, b"AKIA", upper_digit, 16, true) {
        Some("AWS 키")
    } else if [b"ghp_", b"gho_", b"ghu_", b"ghs_", b"ghr_"].iter().any(|prefix| token_after(line, *prefix, alnum, 36, false)) {
        Some("GitHub 토큰")
    } else if token_after(line, b"sk-", url_safe, 32, false) {
        Some("API 키(sk-)")
    } else if [b"xoxa-", b"xoxb-", b"xoxp-", b"xoxr-", b"xoxs-"].iter().any(|prefix| token_after(line, *prefix, dashed, 10, false)) {
        Some("Slack 토큰")
    } else if token_after(line, b"AIza", url_safe, 35, false) {
        Some("Google API 키")
    } else {
        None
    }
}

/// `-----BEGIN (낱말 )*PRIVATE KEY-----` 꼴이 있는가.
fn private_key(line: &[u8]) -> bool {
    const BEGIN: &[u8] = b"-----BEGIN ";
    line.windows(BEGIN.len()).enumerate().filter(|(_, window)| *window == BEGIN).any(|(start, _)| {
        let rest = &line[start + BEGIN.len()..];
        let label = &rest[..rest.iter().take_while(|&&byte| byte.is_ascii_uppercase() || byte == b' ').count()];
        let before = label.strip_suffix(b"PRIVATE KEY".as_slice());
        rest[label.len()..].starts_with(b"-----") && before.is_some_and(|words| words.is_empty() || words.ends_with(b" "))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(source: &str) -> Vec<String> {
        std_paths(&flatten(source).unwrap()).into_iter().map(|(_, path)| path.join("::")).collect()
    }

    #[test]
    fn 표준_라이브러리_경로를_조각마다_찾는다() {
        assert_eq!(paths("use std::fs;"), ["fs"]);
        assert_eq!(paths("use std::{collections::HashMap, io::{self, Read}, time::*};"), ["collections::HashMap", "io::self", "io::Read", "time::*"]);
        assert_eq!(paths("fn f() { ::core::mem::swap(&mut a, &mut b); r#std::fs::read(x); }"), ["mem::swap", "fs::read"]);
    }

    #[test]
    fn 매크로_안에_적은_경로도_찾는다() {
        assert_eq!(paths("fn f() { let v = vec![std::env::var(\"HOME\")]; }"), ["env::var"]);
        assert_eq!(paths("macro_rules! m { () => { std::process::exit(0) } }"), ["process::exit"]);
    }

    #[test]
    fn 경로_없이_이름만_쓴_것은_빈_경로로_나온다() {
        assert_eq!(paths("use std as s; use std::{self as t};"), ["", "self"]);
        assert_eq!(paths("macro_rules! m { ($root:ident) => { $root::fs::read(\"x\") } } fn f() { m!(std); }"), [""]);
    }

    #[test]
    fn 필드_이름과_글자_안의_낱말은_경로로_보지_않는다() {
        assert_eq!(paths("fn f(s: &S) -> u8 { let text = \"std::fs\"; /* std::fs */ s.core }"), Vec::<String>::new());
        // 범위의 점 뒤에 온 것은 필드가 아니라 경로다.
        assert_eq!(paths("fn f() { let _ = (0..std::process::id(), &v[..core::mem::size_of_val(&v)], 1..=alloc::format(x)); }"), ["process::id", "mem::size_of_val", "format"]);
    }

    #[test]
    fn 매크로_호출과_비교_기호를_가린다() {
        let tokens = flatten("fn f() { println!(\"x\"); if a != b { dbg!(a); } }").unwrap();
        let names: Vec<String> = macro_calls(&tokens).into_iter().map(|(_, name)| name).collect();
        assert_eq!(names, ["println", "dbg"]);
    }

    #[test]
    fn 판단이_든_줄을_찾는다() {
        let source = "fn main() {\n    let a = run();\n    let b = a.map(|x| x + 1);\n    if a { stop() } else { go() }\n    let c = &a;\n    for x in y {}\n    let d = a && b;\n    let e = a || b;\n    match a { _ => {} }\n    while a {}\n    loop {}\n    let Some(f) = a else { return };\n}";
        assert_eq!(decisions(&flatten(source).unwrap()), [3, 4, 6, 7, 8, 9, 10, 11, 12]);
        assert_eq!(decisions(&flatten("fn main() -> Result<(), String> {\n    run(&a, b::c)?;\n    Ok(())\n}").unwrap()), Vec::<usize>::new());
        // 약속을 구현하는 머리의 for 는 반복이 아니다. 그 뒤의 몸통에 든 반복은 판단이다.
        let source = "impl Show for Name {\n    fn show(&self, each: impl Fn(&str)) {\n        for part in self.parts() {}\n    }\n}\nfn run(each: impl Fn()) -> impl Sized { for x in y {} }";
        assert_eq!(decisions(&flatten(source).unwrap()), [3, 6]);
    }

    #[test]
    fn 속성의_안쪽_글자와_다른_파일을_여는_선언을_찾는다() {
        let tokens = flatten("#![cfg(test)]\n#[cfg(not(clippy))]\n#[path = \"x.rs\"]\nmod a;\n/// 설명\nmod b {}").unwrap();
        assert_eq!(attributes(&tokens), [(1, "cfg(test)".to_string()), (2, "cfg(not(clippy))".to_string()), (3, "path=_".to_string()), (5, "doc=_".to_string())]);
        assert!(opens_other_files(&tokens));
        assert!(!opens_other_files(&flatten("mod b {} fn f(module: u8) {}").unwrap()));
    }

    #[test]
    fn 시험에서만_쓰는_표시가_모듈_앞에_있는지_가린다() {
        let source = "#[cfg(test)]\nmod tests {}\n#[cfg(test)]\n/// 설명\n#[derive(Debug)]\npub(crate) mod more {}\n#[cfg(test)]\nfn helper() {}\nfn run() {\n    #[cfg(test)]\n    {\n        stop();\n    }\n}\n#[cfg(not(test))]\nmod real {}";
        assert_eq!(test_only(&flatten(source).unwrap()), [(1, true), (3, true), (7, false), (10, false)]);
        assert_eq!(test_only(&flatten("#![cfg(test)]\nmod tests {}").unwrap()), [(1, false)]);
    }

    #[test]
    fn 검사를_끄는_표시의_이름을_꺼낸다() {
        let tokens = flatten("#![allow(dead_code, clippy::too_many_arguments)]\n#[cfg_attr(test, expect(unused, reason = \"x\"))]\n#[derive(Debug)]\n#[warn(warnings, clippy::todo)]\n#[deny(missing_docs)]\nstruct S;").unwrap();
        let expected = [(1, "dead_code"), (1, "clippy::too_many_arguments"), (2, "unused"), (4, "warnings"), (4, "clippy::todo")];
        assert_eq!(allows(&tokens), expected.map(|(line, name)| (line, name.to_string())));
    }

    #[test]
    fn 함수와_시험_표시와_확인하는_줄을_찾는다() {
        let source = "#[derive(Debug)]\nstruct S;\nfn plain() { run(); }\n#[cfg(test)]\nmod tests {\n#[test]\nfn n1_확인한다() { assert!(run()); }\n#[test]\nfn n2_안_한다() { debug_assert!(run()); }\n#[test]\n#[should_panic(expected = \"x\")]\nfn n3_멈춘다() { run(); }\n#[tokio::test]\nasync fn n4_풀어_본다() { run().unwrap(); }\n#[tokio::test(flavor = \"current_thread\")]\nasync fn 인자가_붙은_표시() { run(); }\n#[attest(x)]\nfn 시험이_아니다() {}\n}";
        let found = functions(&flatten(source).unwrap());
        let expected = [
            ("plain", false, false),
            ("n1_확인한다", true, true),
            ("n2_안_한다", true, false),
            ("n3_멈춘다", true, true),
            ("n4_풀어_본다", true, true),
            ("인자가_붙은_표시", true, false),
            ("시험이_아니다", false, false),
        ];
        assert_eq!(found, expected.map(|(name, test, checks)| Function { name: name.to_string(), test, checks }));
        assert_eq!(forbidden_number("n12_x"), Some("N12".to_string()));
        assert_eq!(forbidden_number("n_x"), None);
        assert_eq!(forbidden_number("n1x"), None);
        assert_eq!(forbidden_number("number"), None);
    }

    #[test]
    fn 알려진_형태의_비밀_값을_찾는다() {
        let aws = ["AKIA", "ABCDEFGHIJKLMNOP"].concat();
        assert_eq!(secret_in(format!("key = {aws}").as_bytes()), Some("AWS 키"));
        assert_eq!(secret_in(format!("{aws}Q").as_bytes()), None);
        assert_eq!(secret_in(format!("x{aws}").as_bytes()), None);
        assert_eq!(secret_in(["-----BEGIN RSA PRIVATE", " KEY-----"].concat().as_bytes()), Some("개인 키"));
        assert_eq!(secret_in(["-----BEGIN PRIVATE", " KEY-----"].concat().as_bytes()), Some("개인 키"));
        assert_eq!(secret_in(b"-----BEGIN PUBLIC KEY-----"), None);
        assert_eq!(secret_in(["gh", "p_", &"a".repeat(36)].concat().as_bytes()), Some("GitHub 토큰"));
        assert_eq!(secret_in(["s", "k-", &"a".repeat(32)].concat().as_bytes()), Some("API 키(sk-)"));
        assert_eq!(secret_in(["task-", &"a".repeat(32)].concat().as_bytes()), None);
        assert_eq!(secret_in(["xo", "xb-", "1234567890"].concat().as_bytes()), Some("Slack 토큰"));
        assert_eq!(secret_in(["AI", "za", &"b".repeat(35)].concat().as_bytes()), Some("Google API 키"));
        assert_eq!(secret_in("평범한 줄".as_bytes()), None);
        assert_eq!(secret_in(["AKIA", "ABC"].concat().as_bytes()), None);
    }

    #[test]
    fn 같은_글자가_길게_이어진_줄도_금방_훑는다() {
        // 걸리는 시간이 줄 길이의 제곱으로 늘어나면 이 시험은 몇 분이 걸린다.
        let started = std::time::Instant::now();
        assert_eq!(secret_in("x".repeat(2_000_000).as_bytes()), None);
        assert_eq!(secret_in("AKIA ".repeat(400_000).as_bytes()), None);
        let attributes = functions(&flatten(&"#[a]\n".repeat(50_000)).unwrap());
        assert!(attributes.is_empty() && started.elapsed() < std::time::Duration::from_secs(20));
    }
}
