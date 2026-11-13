//! 조립과 진입점. 통로에서 받은 것을 블록에 넘기고 결과를 내보낸다. 판단은 블록이 한다.

fn main() -> Result<(), String> {
    let name = std::env::args().skip(1).collect::<Vec<_>>().join(" ");
    println!("{}", greet::greet_or_ask(&name, input::line)?);
    Ok(())
}
