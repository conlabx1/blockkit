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
    let line = ["실패: ", "건너뜀: ", "돌지 않음: "].iter().fold(line, |line, head| line.strip_prefix(head).unwrap_or(line));
    bricks.iter().position(|brick| line.starts_with(&format!("{}:", brick.rel)) || line.starts_with(&format!("{}/", brick.rel)) || (brick.rel == "app" && line.starts_with("조립(app):")))
}

const STYLE: &str = r#"
body{margin:0;padding:24px;font-family:-apple-system,"Apple SD Gothic Neo",sans-serif;color:#2a2522;background:#ece9e4}
.verdict{border-radius:4px;padding:16px 20px;color:#fff;background:#6f6a64;box-shadow:inset 8px 0 #3d7a4f}
.verdict.fail{box-shadow:inset 8px 0 #e8462a}.verdict.skip{box-shadow:inset 8px 0 #d9b44a}
.verdict h1{margin:0;font-size:22px}.verdict p{margin:6px 0 0;opacity:.9}
.board{display:flex;flex-wrap:wrap;gap:6px;margin-top:20px}
.board .brick{display:flex;align-items:center;box-sizing:border-box;height:30px;padding:0 14px;border-radius:2px;font-size:13px;font-weight:600;color:#fff;white-space:nowrap}
.board .brick.ok{background:#a4482e;box-shadow:inset 0 -3px rgba(0,0,0,.2)}
.board .brick.fail{background:#2f2622;color:#ffd2c7;box-shadow:inset 0 0 0 2px #e8462a}
.board .brick.skip{background:#f3ead2;color:#5a4521;border:2px dashed #b08d3c}
.legend{display:flex;align-items:center;gap:5px;margin-top:10px;font-size:12px;color:#5a524b}
.legend .brick{box-sizing:border-box;width:22px;height:11px;margin-left:8px;border-radius:1px}
.legend .brick:first-child{margin-left:0}
.site{position:relative;width:max-content;padding:0 64px;margin:50px auto 0}
.house{position:relative;display:flex;flex-direction:column}
.roof{position:relative;z-index:1;display:block;width:calc(100% + 36px);height:84px;margin:0 -18px;overflow:visible}
.roof polygon,.roof polyline{vector-effect:non-scaling-stroke;stroke-linejoin:round}
.roof .tiles{fill:#7a3b2a;stroke:#552618;stroke-width:2}
.roof .shingle{fill:none;stroke:#5a2a1d;stroke-width:1.5}
.roof .frame{fill:none;stroke:#c9a66b;stroke-width:3}
.chimney{position:absolute;z-index:0;top:4px;left:70%;width:20px;height:52px;background:linear-gradient(#4a3b34 0 5px,transparent 5px),repeating-linear-gradient(180deg,#9a4229 0 8px,#b9b0a3 8px 10px)}
.smoke{position:absolute;border-radius:50%;background:#b8b4ae}
.smoke:nth-child(1){left:4px;top:-13px;width:12px;height:12px;opacity:.8}
.smoke:nth-child(2){left:9px;top:-28px;width:16px;height:16px;opacity:.6}
.smoke:nth-child(3){left:18px;top:-47px;width:21px;height:21px;opacity:.4}
.walls{display:flex;gap:3px;padding:3px;background:#b9b0a3}
.section{display:flex;flex-direction:column;gap:3px}
.stack{display:flex;flex-direction:column-reverse;gap:3px}
.row{display:flex;gap:3px}
.row .brick{box-sizing:border-box;width:34px;height:16px;border-radius:1px;font-size:9px;font-weight:700;line-height:16px;text-align:center;color:#ffe3d6}
.row .brick.half{width:15.5px}
.row .brick.fill{background:#a4482e}
.row .brick.fill:nth-child(3n){background:#974027}
.row .brick.fill:nth-child(4n+2){background:#b0553a}
.row .brick.ok,.legend .brick.ok{background:#b5532f;box-shadow:inset 0 -2px rgba(0,0,0,.2)}
.row .brick.fail,.legend .brick.fail{background:#2f2622;box-shadow:inset 0 3px 5px rgba(0,0,0,.7);color:#ff8a70}
.row .brick.skip,.legend .brick.skip{background:#efe6d0;border:1.5px dashed #8a6d3b;color:#6b5532;line-height:13px}
.jamb{width:15.5px;background:linear-gradient(180deg,#a4482e 0 16px,#b9b0a3 16px 19px,#a4482e 19px)}
.window{box-sizing:border-box;width:108px;height:35px;border:3px solid #6e4b30}
.window.glass{background:linear-gradient(90deg,transparent 50px,#6e4b30 50px 52px,transparent 52px),linear-gradient(180deg,transparent 13px,#6e4b30 13px 15px,transparent 15px),linear-gradient(160deg,#dcecf7,#9fc3dd)}
.site.done .window.glass{background:linear-gradient(90deg,transparent 50px,#6e4b30 50px 52px,transparent 52px),linear-gradient(180deg,transparent 13px,#6e4b30 13px 15px,transparent 15px),linear-gradient(160deg,#fff4c2,#f2c25a);box-shadow:0 0 12px rgba(242,194,90,.8)}
.window.hole{background:#2a221e;border:2px dashed #7a685a;box-shadow:inset 0 4px 8px rgba(0,0,0,.7)}
.section>.name{box-sizing:border-box;max-width:145px;height:16px;padding:0 5px;border:1.5px solid #8c7b62;border-radius:2px;background:#efe8da;font-size:10px;font-weight:700;line-height:13px;color:#3a2f2a;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.name.skip{border-style:dashed}
.name.fail{border-color:#e8462a;background:#fde4dc;color:#b3260f}
.porch{position:relative;width:52px;background:linear-gradient(#b9b0a3 0 19px,transparent 19px),repeating-linear-gradient(0deg,#a4482e 0 16px,#b9b0a3 16px 19px)}
.porch::before{content:"";position:absolute;bottom:0;left:9px;box-sizing:border-box;width:34px;height:54px;border:3px solid #4a3222;border-bottom:0;border-radius:4px 4px 0 0;background:#6b4a33}
.porch::after{content:"";position:absolute;bottom:24px;left:34px;width:4px;height:4px;border-radius:50%;background:#e8c86a}
.base{position:relative;display:flex;align-items:center;justify-content:center;height:20px;margin:0 -8px;background:#aaa59d;box-shadow:inset 0 2px rgba(255,255,255,.35),inset 0 -2px rgba(0,0,0,.15)}
.base.formwork{background:repeating-linear-gradient(90deg,#c99b5f 0 24px,#9c7340 24px 26px);box-shadow:none}
.base>.name{position:relative;z-index:1;padding:0 6px;border-radius:2px;background:rgba(255,255,255,.55);font-size:10px;font-weight:700;line-height:14px;color:#3a2f2a}
.base>.name.skip{outline:1.5px dashed #8a6d3b}
.base>.name.fail{background:#fde4dc;color:#b3260f;outline:1.5px solid #e8462a}
.crack{position:absolute;inset:0;background:url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 100 20' preserveAspectRatio='none'%3E%3Cpolyline points='22,0 26,7 21,12 27,20' fill='none' stroke='%23e8462a' stroke-width='2'/%3E%3Cpolyline points='76,0 72,8 78,13 73,20' fill='none' stroke='%23e8462a' stroke-width='2'/%3E%3C/svg%3E") center/100% 100% no-repeat}
.scaffold{position:absolute;top:24px;bottom:20px;width:46px;background:linear-gradient(#7d8288,#7d8288) 0 0/4px 100% no-repeat,linear-gradient(#7d8288,#7d8288) 100% 0/4px 100% no-repeat,linear-gradient(to top right,transparent calc(50% - 1px),#9aa0a6 calc(50% - 1px) calc(50% + 1px),transparent calc(50% + 1px)),repeating-linear-gradient(180deg,transparent 0 30px,#b07a3f 30px 36px)}
.scaffold.left{left:10px}.scaffold.right{right:10px}
.ground{position:relative;height:20px;margin:0 -64px;background:#a3896c}
.site.done .ground{background:linear-gradient(#6fa85a 0 7px,#8b6b4a 7px)}
.yard{position:absolute;left:10px;right:10px;bottom:100%;display:flex;align-items:flex-end;gap:5px}
.yard .flower:nth-child(3){margin-left:auto}
.tree{width:32px;height:50px;background:radial-gradient(circle at 50% 36%,#5f9e4b 0 15px,transparent 16px),linear-gradient(90deg,transparent 14px,#6b4a33 14px 18px,transparent 18px)}
.flower{width:8px;height:13px;background:radial-gradient(circle at 50% 4px,#e8628a 0 3px,transparent 3.5px),linear-gradient(90deg,transparent 3px,#4f8a3c 3px 5px,transparent 5px)}
.note{margin-top:16px;padding:12px 16px;border-radius:4px;background:#fff;border-left:8px solid #e8462a}
.note h2{margin:0 0 6px;font-size:16px}.note ul{margin:0;padding-left:18px}
.note li{margin:4px 0;white-space:pre-wrap;word-break:break-all}
.gate{display:inline-block;padding:0 6px;border-radius:4px;background:#eee;font-size:12px}
"#;

const FILL: &str = "<span class=\"brick fill\"></span>";
const HALF: &str = "<span class=\"brick fill half\"></span>";
const TILES: &str = "<polygon class=\"tiles\" points=\"50,1 99,29 1,29\"/><polyline class=\"shingle\" points=\"37.75,8 62.25,8\"/><polyline class=\"shingle\" points=\"25.5,15 74.5,15\"/><polyline class=\"shingle\" points=\"13.25,22 86.75,22\"/>";
const FRAME: &str = "<polyline class=\"frame\" points=\"1,29 50,1 99,29 1,29\"/><polyline class=\"frame\" points=\"50,1 50,29\"/><polyline class=\"frame\" points=\"25.5,15 50,29 74.5,15\"/>";
const SCAFFOLD: &str = "<span class=\"scaffold left\"></span><span class=\"scaffold right\"></span>";
const CHIMNEY: &str = "<span class=\"chimney\"><i class=\"smoke\"></i><i class=\"smoke\"></i><i class=\"smoke\"></i></span>";
const YARD: &str = "<div class=\"yard\"><i class=\"tree\"></i><i class=\"flower\"></i><i class=\"flower\"></i><i class=\"tree\"></i></div>";

fn worst<'a>(statuses: impl Iterator<Item = &'a str>) -> &'static str {
    let classes: Vec<&str> = statuses.map(|status| state(status).0).collect();
    ["fail", "skip"].into_iter().find(|class| classes.contains(class)).unwrap_or("ok")
}

pub fn page(bricks: &[Brick], gates: &[(&str, Gate)], code: u8, output: &str, at: u64) -> String {
    let mut owned: Vec<Vec<(&str, &str, &str)>> = vec![Vec::new(); bricks.len() + 1];
    for (name, gate) in gates {
        if gate.status != PASS && gate.details.is_empty() {
            owned[bricks.len()].push((name, gate.status, &gate.summary));
        }
        let mut last = bricks.len();
        for line in &gate.details {
            if !line.starts_with(char::is_whitespace) {
                last = owner(line, bricks).unwrap_or(bricks.len());
            }
            owned[last].push((name, gate.status, line));
        }
    }
    let door = |index: usize| bricks[index].rel.starts_with("doors/");
    let walls: Vec<usize> = (0..bricks.len()).filter(|&index| bricks[index].rel != "app").collect();
    let rows_for = |rules: usize| 1 + (0..).take_while(|&rows| (0..=rows).map(|row| 4 - row % 2).sum::<usize>() < rules).count();
    let height = walls.iter().map(|&index| rows_for(bricks[index].rules.len()) + if door(index) { 2 } else { 0 }).fold(3, usize::max);
    let section = |index: usize| {
        let brick = &bricks[index];
        let lines = &owned[index];
        let class = worst(lines.iter().map(|line| line.1));
        let mut ids = brick.rules.iter().map(|id| {
            let rule = worst(lines.iter().filter(|line| line.2.contains(&format!("[{id}]")) || line.2.contains(&format!(": {}_", id.to_lowercase()))).map(|line| line.1));
            format!("<span class=\"brick {rule}\" data-id=\"{}\">{}</span>", escape(id), escape(id))
        });
        let mut stack = String::new();
        for row in 0..height - if door(index) { 2 } else { 0 } {
            let cells: String = (0..4 - row % 2).map(|_| ids.next().unwrap_or_else(|| FILL.to_string())).collect();
            stack += &if row % 2 == 1 { format!("<div class=\"row shift\">{HALF}{cells}{HALF}</div>") } else { format!("<div class=\"row\">{cells}</div>") };
        }
        if door(index) {
            stack += &format!("<div class=\"row pane\"><span class=\"jamb\"></span><span class=\"window {}\"></span><span class=\"jamb\"></span></div>", if class == "ok" { "glass" } else { "hole" });
        }
        format!("<div class=\"section{} {class}\"><span class=\"brick name {class}\" data-unit=\"{}\">{}</span><div class=\"stack\">{stack}</div></div>", if door(index) { " door" } else { "" }, escape(&brick.rel), escape(&brick.label))
    };
    let (overall, stage, roof) = match code {
        0 => ("ok", "done", TILES),
        2 => ("skip", "pale", FRAME),
        _ => ("fail", "open", ""),
    };
    let done = code == 0;
    let wall: String = walls.iter().map(|&index| section(index)).collect::<String>() + if done { "<span class=\"porch\"></span>" } else { "" };
    let base: String = (0..bricks.len())
        .filter(|&index| bricks[index].rel == "app")
        .map(|index| {
            let class = worst(owned[index].iter().map(|line| line.1));
            let (kind, crack) = if class == "fail" { ("formwork", "<i class=\"crack\"></i>") } else { ("poured", "") };
            format!("<div class=\"base {kind}\"><span class=\"brick name {class}\" data-unit=\"{}\">{}</span>{crack}</div>", escape(&bricks[index].rel), escape(&bricks[index].label))
        })
        .collect();
    let site = format!(
        "<div class=\"site {stage}\">{}<div class=\"house\">{}<svg class=\"roof {stage}\" viewBox=\"0 0 100 30\" preserveAspectRatio=\"none\">{roof}</svg><div class=\"walls\">{wall}</div>{base}</div><div class=\"ground\">{}</div></div>",
        if stage == "open" { SCAFFOLD } else { "" },
        if done { CHIMNEY } else { "" },
        if done { YARD } else { "" }
    );
    let board: String = gates
        .iter()
        .map(|(name, gate)| {
            let (class, mark) = state(gate.status);
            format!("<span title=\"{}\" class=\"brick {class}\">{mark} {}</span>", escape(&gate.summary), escape(name))
        })
        .collect();
    let note = |key: &str, title: &str, lines: &[(&str, &str, &str)]| {
        if lines.is_empty() {
            return String::new();
        }
        let items: String = lines.iter().map(|(gate, _, line)| format!("<li><span class=\"gate\">{}</span> {}</li>", escape(gate), escape(&crate::clean(line)))).collect();
        format!("<section class=\"note\" data-note=\"{}\"><h2>{}</h2><ul>{items}</ul></section>", escape(key), escape(title))
    };
    let notes: String = bricks.iter().enumerate().map(|(index, brick)| note(&brick.rel, &brick.label, &owned[index])).collect::<String>() + &note("project", "틀 전체", &owned[bricks.len()]);
    let verdict = output.lines().find(|line| line.starts_with("결과:")).unwrap_or("결과: 실패");
    format!(
        "<!doctype html><html lang=\"ko\"><head><meta charset=\"utf-8\"><meta http-equiv=\"refresh\" content=\"3\"><title>blockkit 점검</title><style>{STYLE}</style></head><body><header class=\"verdict {overall}\"><h1>{}</h1><p id=\"time\" data-at=\"{at}\"></p></header><div class=\"board\">{board}</div><div class=\"legend\"><span class=\"brick ok\"></span>통과<span class=\"brick fail\"></span>실패<span class=\"brick skip\"></span>점검 못 함</div>{site}{notes}<script>const t=document.getElementById(\"time\"),at=new Date(t.dataset.at*1000);t.textContent=\"마지막 점검 \"+at.toLocaleString(\"ko-KR\")+\" · \"+Math.max(0,Math.round((Date.now()-at)/1000))+\"초 전\";const s=document.querySelector(\".site\");s.style.zoom=Math.min(1.6,(document.body.clientWidth-48)/s.offsetWidth);</script></body></html>\n",
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

    fn section(html: &str, unit: &str) -> String {
        let at = html.find(&format!(r#"data-unit="{unit}""#)).unwrap();
        let start = html[..at].rfind(r#"<div class="section"#).unwrap();
        let rest = &html[start + 1..];
        let end = [rest.find(r#"<div class="section"#), rest.find(r#"<div class="base"#)].into_iter().flatten().min().unwrap_or(rest.len());
        html[start..start + 1 + end].to_string()
    }

    #[test]
    fn 실패한_줄은_그_블록과_그_금지_사항_벽돌만_빈자리로_바꾼다() {
        let results = [("금지 연결", gate(FAIL, &["blocks/greet: 금지 사항 [N2] 을 확인하는 시험이 없습니다.", "  이어지는 줄"])), ("카드", gate(PASS, &[])), ("의존성", gate(SKIP, &["blocks/greet: [N2] 점검 못 함"]))];
        let html = page(&sample(), &results, 1, "블록 1개\n결과: 실패 1개\n", 0);
        assert!(html.contains(r#"<header class="verdict fail"><h1>결과: 실패 1개</h1>"#), "{html}");
        let greet = section(&html, "blocks/greet");
        assert!(greet.starts_with(r#"<div class="section fail">"#) && greet.contains(r#"class="brick name fail" data-unit="blocks/greet""#), "{html}");
        assert!(greet.contains(r#"class="brick fail" data-id="N2""#) && greet.contains(r#"class="brick ok" data-id="N1""#), "{html}");
        let other = section(&html, "blocks/greet2");
        assert!(other.starts_with(r#"<div class="section ok">"#) && other.contains(r#"class="brick name ok" data-unit="blocks/greet2""#) && other.contains(r#"class="brick ok" data-id="N2""#), "{html}");
        assert!(section(&html, "doors/input").starts_with(r#"<div class="section door ok">"#), "{html}");
        assert!(html.contains(r#"<div class="base poured"><span class="brick name ok" data-unit="app">app</span></div>"#), "{html}");
        let notes = html.split(r#"data-note="blocks/greet""#).nth(1).unwrap();
        assert!(notes.split("</section>").next().unwrap().contains("이어지는 줄"), "{html}");
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
    }

    #[test]
    fn 점검하지_못한_금지_사항은_점선_벽돌이고_실패가_없으면_지붕_뼈대만_세우고_마감은_하지_않는다() {
        let results = [("시험", gate(SKIP, &["건너뜀: blocks/greet: n1_빈_이름은_받지_않는다"])), ("의존성", Gate { status: SKIP, summary: "점검하지 못했습니다".to_string(), details: Vec::new() })];
        let html = page(&sample(), &results, 2, "결과: 실패는 없지만 점검하지 못한 항목이 2개 있습니다.", 0);
        let greet = section(&html, "blocks/greet");
        assert!(greet.starts_with(r#"<div class="section skip">"#) && greet.contains(r#"class="brick name skip""#), "{html}");
        assert!(greet.contains(r#"class="brick skip" data-id="N1""#) && greet.contains(r#"class="brick ok" data-id="N2""#), "{html}");
        assert!(html.contains(r#"class="verdict skip""#) && html.contains(r#"<div class="site pale">"#) && html.contains(r#"<svg class="roof pale""#) && html.contains(r#"class="frame""#), "{html}");
        for part in [r#"class="scaffold"#, r#"class="tiles""#, r#"class="chimney""#, r#"class="smoke""#, r#"class="porch""#, r#"class="yard""#] {
            assert!(!html.contains(part), "{part} {html}");
        }
        assert!(html.contains(r#"class="base poured""#) && section(&html, "doors/input").contains(r#"class="window glass""#), "{html}");
    }

    #[test]
    fn 실패가_있으면_지붕_없이_비계를_세우고_조립이_실패하면_기초는_금_간_거푸집이다() {
        let html = page(&sample(), &[("시험", gate(FAIL, &["실패: 조립(app): 전체가_돈다"]))], 1, "결과: 실패 1개", 0);
        assert!(html.contains(r#"<div class="site open">"#) && html.contains(r#"<svg class="roof open""#), "{html}");
        assert!(html.contains(r#"class="scaffold left""#) && html.contains(r#"class="scaffold right""#), "{html}");
        for part in [r#"class="tiles""#, r#"class="frame""#, r#"class="chimney""#, r#"class="porch""#, r#"class="yard""#] {
            assert!(!html.contains(part), "{part} {html}");
        }
        assert!(html.contains(r#"<div class="base formwork"><span class="brick name fail" data-unit="app">app</span><i class="crack"></i></div>"#), "{html}");
        let walls = page(&sample(), &[("카드", gate(FAIL, &["blocks/greet: [N1] 깨짐"]))], 1, "결과: 실패 1개", 0);
        assert!(walls.contains(r#"<div class="base poured">"#) && !walls.contains(r#"class="crack""#) && walls.contains(r#"class="scaffold left""#), "{walls}");
    }

    #[test]
    fn 통로가_통과하면_유리_낀_창문이고_아니면_뚫린_구멍만_있다() {
        let broken = page(&sample(), &[("시험", gate(FAIL, &["실패: doors/input: n1_끝나면_알린다"]))], 1, "결과: 실패 1개", 0);
        let door = section(&broken, "doors/input");
        assert!(door.starts_with(r#"<div class="section door fail">"#) && door.contains(r#"class="window hole""#) && !door.contains(r#"class="window glass""#), "{broken}");
        let fine = page(&sample(), &[("시험", gate(FAIL, &["실패: blocks/greet: n1_빈_이름은_받지_않는다"]))], 1, "결과: 실패 1개", 0);
        assert!(section(&fine, "doors/input").contains(r#"class="window glass""#), "{fine}");
        assert!(!section(&fine, "blocks/greet").contains("window"), "{fine}");
        let unchecked = page(&sample(), &[("시험", gate(SKIP, &["건너뜀: doors/input: n1_끝나면_알린다"]))], 2, "결과: 점검 못 함 1개", 0);
        assert!(section(&unchecked, "doors/input").contains(r#"class="window hole""#), "{unchecked}");
    }

    #[test]
    fn 전부_통과하면_지붕과_굴뚝_현관문_마당까지_붙어_집이_완성된다() {
        let html = page(&sample(), &[("카드", gate(PASS, &[]))], 0, "결과: 전부 통과", 0);
        assert!(html.contains(r#"<header class="verdict ok">"#) && html.contains(r#"<div class="site done">"#), "{html}");
        assert!(html.contains(r#"<svg class="roof done""#) && html.contains(r#"class="tiles""#), "{html}");
        for part in [r#"class="chimney""#, r#"class="smoke""#, r#"class="porch""#, r#"class="yard""#] {
            assert!(html.contains(part), "{part} {html}");
        }
        assert!(!html.contains(r#"class="scaffold"#) && !html.contains(r#"class="frame""#), "{html}");
        assert!(html.contains(r#"class="base poured""#) && section(&html, "doors/input").contains(r#"class="window glass""#), "{html}");
        assert!(html.contains(r#"<div class="legend"><span class="brick ok"></span>통과<span class="brick fail"></span>실패<span class="brick skip"></span>점검 못 함</div>"#), "{html}");
    }

    #[test]
    fn 벽은_금지_사항_벽돌과_채움_벽돌이_엇갈려_쌓이고_블록이_늘면_옆으로_넓어진다() {
        let brick = |rel: &str, rules: usize| Brick { rel: rel.to_string(), label: rel.to_string(), rules: (1..=rules).map(|number| format!("N{number}")).collect() };
        let many = vec![brick("blocks/a", 12), brick("blocks/b", 0), brick("blocks/c", 2), brick("blocks/d", 1), brick("doors/e", 1), brick("app", 0)];
        let html = page(&many, &[], 0, "결과: 전부 통과", 0);
        assert_eq!(html.matches(r#"<div class="section"#).count(), 5, "{html}");
        let height = |unit: &str| section(&html, unit).matches(r#"<div class="row"#).count() + section(&html, unit).matches(r#"<div class="row pane""#).count();
        assert_eq!(height("blocks/a"), 4, "{html}");
        for unit in ["blocks/b", "blocks/c", "blocks/d", "doors/e"] {
            assert_eq!(height(unit), 4, "{unit} {html}");
        }
        let a = section(&html, "blocks/a");
        assert_eq!(a.matches("data-id=").count(), 12, "{html}");
        assert!(a.contains(r#"<div class="row"><span class="brick ok" data-id="N1">N1</span>"#), "{html}");
        assert!(a.contains(r#"<div class="row shift"><span class="brick fill half"></span><span class="brick ok" data-id="N5">N5</span><span class="brick ok" data-id="N6">N6</span><span class="brick ok" data-id="N7">N7</span><span class="brick fill half"></span></div>"#), "{html}");
        let b = section(&html, "blocks/b");
        assert!(b.contains(r#"<span class="brick fill"></span>"#) && !b.contains("data-id="), "{html}");
        let rows = |html: &str, unit: &str| section(html, unit).matches(r#"<div class="row"#).count() + section(html, unit).matches(r#"<div class="row pane""#).count();
        let low = page(&[brick("blocks/f", 1), brick("app", 0)], &[], 0, "결과: 전부 통과", 0);
        assert_eq!(rows(&low, "blocks/f"), 3, "{low}");
        let full = page(&[brick("blocks/f", 11), brick("app", 0)], &[], 0, "결과: 전부 통과", 0);
        assert_eq!(rows(&full, "blocks/f"), 3, "{full}");
        let door = page(&[brick("doors/g", 5), brick("app", 0)], &[], 0, "결과: 전부 통과", 0);
        assert_eq!((rows(&door, "doors/g"), section(&door, "doors/g").matches("data-id=").count()), (4, 5), "{door}");
    }

    #[test]
    fn 시험_관문의_실패_건너뜀_돌지_않음_줄도_제_벽돌과_그_금지_사항_벽돌에_붙는다() {
        let results = [(
            "시험",
            gate(FAIL, &["실패: blocks/greet: n1_빈_이름은_받지_않는다", "건너뜀: doors/input: n1_끝나면_알린다", "돌지 않음: 조립(app): 전체가_돈다 (app/tests/run.rs). 파일이 mod 선언으로 이어져 있는지 봅니다."]),
        )];
        let html = page(&sample(), &results, 1, "결과: 실패 1개", 0);
        let rules = |unit: &str| section(&html, unit);
        for unit in ["blocks/greet", "doors/input", "app"] {
            assert!(html.contains(&format!(r#"class="brick name fail" data-unit="{unit}""#)), "{html}");
        }
        assert!(html.contains(r#"class="brick name ok" data-unit="blocks/greet2""#), "{html}");
        assert!(rules("blocks/greet").contains(r#"class="brick fail" data-id="N1""#) && rules("blocks/greet").contains(r#"class="brick ok" data-id="N2""#), "{html}");
        assert!(rules("doors/input").contains(r#"class="brick fail" data-id="N1""#), "{html}");
        assert!(!html.contains(r#"data-note="project""#), "{html}");
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
