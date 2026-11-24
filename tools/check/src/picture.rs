use crate::gates::{Gate, PASS, Project, SKIP, forbidden_ids};
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct Brick {
    pub rel: String,
    pub label: String,
    pub rules: Vec<String>,
}

pub fn bricks(project: &Project) -> Vec<Brick> {
    let mut all: Vec<Brick> = project
        .units
        .iter()
        .map(|unit| Brick { rel: unit.rel(), label: format!("{} {}", if unit.kind == "doors" { "통로" } else { "블록" }, unit.name), rules: forbidden_ids(&project.card(unit)) })
        .collect();
    all.push(Brick { rel: "app".to_string(), label: "조립 app".to_string(), rules: Vec::new() });
    all
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for letter in text.chars() {
        match letter {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

fn state(status: &str) -> (&'static str, &'static str) {
    match status {
        PASS => ("ok", "✓"),
        SKIP => ("skip", "!"),
        _ => ("fail", "✕"),
    }
}

fn owner(line: &str, bricks: &[Brick]) -> Option<usize> {
    bricks.iter().position(|brick| line.starts_with(&format!("{}:", brick.rel)) || line.starts_with(&format!("{}/", brick.rel)) || (brick.rel == "app" && line.starts_with("조립(app):")))
}

const STYLE: &str = r#"
body{margin:0;padding:24px;font-family:-apple-system,"Apple SD Gothic Neo",sans-serif;color:#2a2522;background:#ece9e4}
.verdict{border-radius:4px;padding:16px 20px;color:#fff;background:#6f6a64;box-shadow:inset 8px 0 #3d7a4f}
.verdict.fail{box-shadow:inset 8px 0 #e8462a}.verdict.skip{box-shadow:inset 8px 0 #d9b44a}
.verdict h1{margin:0;font-size:22px}.verdict p{margin:6px 0 0;opacity:.9}
.board{display:flex;flex-wrap:wrap;gap:6px;margin-top:20px}
.house{width:min(760px,100%);margin-top:24px}
.roof{display:block;width:100%;height:110px}
.roof polygon{vector-effect:non-scaling-stroke;stroke-width:1}
.roof.done polygon{fill:#4f4039;stroke:#3a2f2a}
.roof.pale polygon{fill:#e5d5a6;stroke:#c9b98a}
.roof.open polygon{fill:none;stroke:#8d867c;stroke-width:2;stroke-dasharray:6 5}
.wall{display:flex;flex-direction:column;gap:6px;padding:6px;background:#c4bdb1}
.course{display:flex;gap:6px}
.base{padding:6px;background:#9a958e}
.brick{display:flex;align-items:center;justify-content:center;box-sizing:border-box;height:42px;min-width:70px;padding:0 14px;border-radius:2px;font-size:14px;font-weight:600;color:#fff;white-space:nowrap}
.brick.name{flex:1;justify-content:flex-start}
.brick.half{width:28px;min-width:28px;padding:0}
.brick.ok{background:#a4482e linear-gradient(180deg,rgba(255,255,255,.1),rgba(0,0,0,.14));box-shadow:inset 0 -3px rgba(0,0,0,.2)}
.brick.fail{background:#2c2624 url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 100 40' preserveAspectRatio='none'%3E%3Cpolyline points='18,0 30,13 22,21 40,31 34,40' fill='none' stroke='%23e8462a' stroke-width='2.5'/%3E%3Cpolyline points='74,0 66,14 80,24 71,40' fill='none' stroke='%23e8462a' stroke-width='2'/%3E%3C/svg%3E") center/100% 100% no-repeat;color:#ffd2c7;box-shadow:inset 0 0 0 2px #e8462a}
.brick.skip{background:#e5d5a6;color:#3a2f1f}
.door .brick.name.ok{background:#8fb4d1 linear-gradient(90deg,transparent 49%,#5d4a3a 49%,#5d4a3a 51%,transparent 51%);color:#14324a;box-shadow:inset 0 0 0 5px #5d4a3a}
.base .brick.ok{background:#85807a;box-shadow:inset 0 -3px rgba(0,0,0,.2)}
.board .brick{height:30px;min-width:0;font-size:13px}
.note{margin-top:16px;padding:12px 16px;border-radius:4px;background:#fff;border-left:8px solid #e8462a}
.note h2{margin:0 0 6px;font-size:16px}.note ul{margin:0;padding-left:18px}
.note li{margin:4px 0;white-space:pre-wrap;word-break:break-all}
.gate{display:inline-block;padding:0 6px;border-radius:4px;background:#eee;font-size:12px}
"#;

pub fn page(bricks: &[Brick], gates: &[(&str, Gate)], code: u8, output: &str, at: u64) -> String {
    let mut owned: Vec<Vec<(&str, &str)>> = vec![Vec::new(); bricks.len() + 1];
    for (name, gate) in gates {
        if gate.status != PASS && gate.details.is_empty() {
            owned[bricks.len()].push((name, &gate.summary));
        }
        let mut last = bricks.len();
        for line in &gate.details {
            if !line.starts_with(char::is_whitespace) {
                last = owner(line, bricks).unwrap_or(bricks.len());
            }
            owned[last].push((name, line));
        }
    }
    let course = |index: usize, offset: bool| {
        let brick = &bricks[index];
        let lines = &owned[index];
        let class = if lines.is_empty() { "ok" } else { "fail" };
        let kind = if brick.rel == "app" { " base" } else if brick.rel.starts_with("doors/") { " door" } else { "" };
        let half = if offset { "<span class=\"brick half ok\"></span>" } else { "" };
        let items: String = brick
            .rules
            .iter()
            .map(|id| {
                let bad = lines.iter().any(|(_, line)| line.contains(&format!("[{id}]")));
                format!("<span class=\"brick {}\" data-id=\"{}\">{}</span>", if bad { "fail" } else { "ok" }, escape(id), escape(id))
            })
            .collect();
        format!("<div class=\"course{kind}\">{half}<span class=\"brick name {class}\" data-unit=\"{}\">{}</span>{items}</div>", escape(&brick.rel), escape(&brick.label))
    };
    let wall: String = (0..bricks.len()).filter(|&index| bricks[index].rel != "app").enumerate().map(|(row, index)| course(index, row % 2 == 1)).collect();
    let base: String = (0..bricks.len()).filter(|&index| bricks[index].rel == "app").map(|index| course(index, false)).collect();
    let board: String = gates
        .iter()
        .map(|(name, gate)| {
            let (class, mark) = state(gate.status);
            format!("<span title=\"{}\" class=\"brick {class}\">{mark} {}</span>", escape(&gate.summary), escape(name))
        })
        .collect();
    let note = |key: &str, title: &str, lines: &[(&str, &str)]| {
        if lines.is_empty() {
            return String::new();
        }
        let items: String = lines.iter().map(|(gate, line)| format!("<li><span class=\"gate\">{}</span> {}</li>", escape(gate), escape(&crate::clean(line)))).collect();
        format!("<section class=\"note\" data-note=\"{}\"><h2>{}</h2><ul>{items}</ul></section>", escape(key), escape(title))
    };
    let notes: String = bricks.iter().enumerate().map(|(index, brick)| note(&brick.rel, &brick.label, &owned[index])).collect::<String>() + &note("project", "틀 전체", &owned[bricks.len()]);
    let verdict = output.lines().find(|line| line.starts_with("결과:")).unwrap_or("결과: 실패");
    let (overall, roof) = match code {
        0 => ("ok", "done"),
        2 => ("skip", "pale"),
        _ => ("fail", "open"),
    };
    format!(
        "<!doctype html><html lang=\"ko\"><head><meta charset=\"utf-8\"><meta http-equiv=\"refresh\" content=\"3\"><title>blockkit 점검</title><style>{STYLE}</style></head><body><header class=\"verdict {overall}\"><h1>{}</h1><p id=\"time\" data-at=\"{at}\"></p></header><div class=\"board\">{board}</div><div class=\"house\"><svg class=\"roof {roof}\" viewBox=\"0 0 100 30\" preserveAspectRatio=\"none\"><polygon points=\"50,1 99,29 1,29\"/></svg><div class=\"wall\">{wall}</div>{base}</div>{notes}<script>const t=document.getElementById(\"time\"),at=new Date(t.dataset.at*1000);t.textContent=\"마지막 점검 \"+at.toLocaleString(\"ko-KR\")+\" · \"+Math.max(0,Math.round((Date.now()-at)/1000))+\"초 전\";</script></body></html>\n",
        escape(verdict)
    )
}

pub fn save(root: &Path, bricks: &[Brick], gates: &[(&str, Gate)], code: u8, output: &str) {
    let at = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |time| time.as_secs());
    let dir = root.join("target");
    let draft = dir.join(format!("check.html.{}", std::process::id()));
    let written = fs::create_dir_all(&dir).and_then(|()| fs::write(&draft, page(bricks, gates, code, output, at))).and_then(|()| fs::rename(&draft, dir.join("check.html")));
    if written.is_err() {
        let _ = fs::remove_file(draft);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gates::FAIL;

    fn gate(status: &'static str, details: &[&str]) -> Gate {
        Gate { status, summary: format!("{status} 요약"), details: details.iter().map(|line| line.to_string()).collect() }
    }

    fn sample() -> Vec<Brick> {
        let brick = |rel: &str, rules: &[&str]| Brick { rel: rel.to_string(), label: rel.to_string(), rules: rules.iter().map(|id| id.to_string()).collect() };
        vec![brick("blocks/greet", &["N1", "N2"]), brick("blocks/greet2", &["N2"]), brick("doors/input", &["N1"]), brick("app", &[])]
    }

    #[test]
    fn 실패한_줄은_그_블록과_그_금지_사항_벽돌만_금이_간_벽돌로_바꾼다() {
        let results = [("금지 연결", gate(FAIL, &["blocks/greet: 금지 사항 [N2] 을 확인하는 시험이 없습니다.", "  이어지는 줄"])), ("카드", gate(PASS, &[]))];
        let html = page(&sample(), &results, 1, "블록 1개\n결과: 실패 1개\n", 0);
        assert!(html.contains(r#"<header class="verdict fail"><h1>결과: 실패 1개</h1>"#), "{html}");
        assert!(html.contains(r#"<svg class="roof open""#), "{html}");
        let door = html.split(r#"<div class="course door">"#).nth(1).unwrap();
        assert!(door.split("</div>").next().unwrap().contains(r#"class="brick name ok" data-unit="doors/input""#), "{html}");
        let base = html.split(r#"<div class="course base">"#).nth(1).unwrap();
        assert!(base.split("</div>").next().unwrap().contains(r#"data-unit="app""#), "{html}");
        assert!(html.contains(r#"class="brick name fail" data-unit="blocks/greet""#), "{html}");
        assert!(html.contains(r#"class="brick name ok" data-unit="blocks/greet2""#), "{html}");
        assert!(html.contains(r#"class="brick name ok" data-unit="app""#), "{html}");
        assert!(html.contains(r#"class="brick fail" data-id="N2""#), "{html}");
        assert!(html.contains(r#"class="brick ok" data-id="N2""#), "{html}");
        assert!(html.contains(r#"class="brick ok" data-id="N1""#), "{html}");
        let greet = html.split(r#"data-note="blocks/greet""#).nth(1).unwrap();
        assert!(greet.split("</section>").next().unwrap().contains("이어지는 줄"), "{html}");
        assert!(html.contains(r#"class="brick fail">✕ 금지 연결"#), "{html}");
        assert!(html.contains(r#"class="brick ok">✓ 카드"#), "{html}");
        assert!(!html.contains(r#"data-note="project""#), "{html}");
    }

    #[test]
    fn 파일_경로로_시작하는_줄과_조립의_줄도_제_벽돌에_붙고_나머지는_틀_전체로_간다() {
        let results = [
            ("패턴", gate(FAIL, &["blocks/greet2/src/lib.rs:3 금지된 것", "조립(app): 시험_이름"])),
            ("비밀 값", gate(FAIL, &["README.md:1 개인 키"])),
            ("의존성", Gate { status: SKIP, summary: "점검하지 못했습니다: 인터넷".to_string(), details: Vec::new() }),
        ];
        let html = page(&sample(), &results, 1, "결과: 실패 2개", 0);
        assert!(html.contains(r#"class="brick name ok" data-unit="blocks/greet""#), "{html}");
        assert!(html.contains(r#"class="brick name fail" data-unit="blocks/greet2""#), "{html}");
        assert!(html.contains(r#"class="brick name fail" data-unit="app""#), "{html}");
        let project = html.split(r#"data-note="project""#).nth(1).unwrap();
        assert!(project.contains("README.md:1 개인 키"), "{html}");
        assert!(project.contains("점검하지 못했습니다: 인터넷"), "{html}");
        assert!(!project.contains("금지된 것"), "{html}");
        assert!(html.contains(r#"class="brick skip">! 의존성"#), "{html}");
        let unchecked = page(&sample(), &results[2..], 2, "결과: 실패는 없지만 점검하지 못한 항목이 1개 있습니다.", 0);
        assert!(unchecked.contains(r#"class="verdict skip""#) && unchecked.contains(r#"<svg class="roof pale""#), "{unchecked}");
        let done = page(&sample(), &[], 0, "결과: 전부 통과", 0);
        assert!(done.contains(r#"class="verdict ok""#) && done.contains(r#"<svg class="roof done""#), "{done}");
    }

    #[test]
    fn 바깥에서_온_글자는_페이지의_코드로_읽히지_않는다() {
        let results = [("패턴", gate(FAIL, &["blocks/greet/src/lib.rs:3 <script>alert(1)</script> & \"x\" 'y'"]))];
        let html = page(&sample(), &results, 1, "결과: 실패 1개 <b>", 0);
        assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt; &amp; &quot;x&quot; &#39;y&#39;"), "{html}");
        assert!(!html.contains("<script>alert"), "{html}");
        assert!(html.contains("결과: 실패 1개 &lt;b&gt;"), "{html}");
    }

    #[test]
    fn 자세한_줄의_제어_글자와_숨는_글자는_물음표로_바꾼다() {
        let results = [("패턴", gate(FAIL, &["blocks/greet/src/lib.rs:3 가\u{1b}[2K나\u{202e}다"]))];
        let html = page(&sample(), &results, 1, "결과: 실패 1개", 0);
        assert!(html.contains("가?[2K나?다"), "{html}");
        assert!(!html.contains('\u{1b}') && !html.contains('\u{202e}'), "{html}");
    }
}
