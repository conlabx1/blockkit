"""조립과 진입점. 통로에서 받은 것을 블록에 넘기고 결과를 내보낸다. 판단은 블록이 한다."""

import sys

from blocks.greet.block import greet


def main(argv: list[str]) -> int:
    try:
        print(greet(" ".join(argv[1:])))
    except ValueError as error:
        print(f"오류: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
