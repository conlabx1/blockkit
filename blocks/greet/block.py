"""이름을 받아 인사말 한 줄을 만든다. 약속은 CARD.md 에 있다."""


def greet(name: str) -> str:
    name = name.strip()
    if not name:
        raise ValueError("이름이 비어 있습니다.")
    if "\n" in name or "\r" in name:
        raise ValueError("이름에 줄바꿈이 들어 있습니다.")
    return f"{name}님, 안녕하세요."
