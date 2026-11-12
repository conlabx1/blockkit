"""점검 명령(check) 자체를 검증한다. 임시 폴더에 작은 프로젝트를 만들고 위반을 하나씩 심어서 실패가 나오는지 본다.

실행: uv run python -m unittest tests.test_check
check 를 고쳤을 때 돌린다. check 의 「시험」 관문은 이 파일을 돌리지 않는다(자기 자신을 다시 부르게 되기 때문이다).
이 시험은 프로젝트의 블록에 기대지 않는다. 예시 블록을 지우거나 바꿔도 그대로 돈다.
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BLOCK = "blocks/sample/code.py"
TEST = "blocks/sample/test_code.py"
CARD = "blocks/sample/CARD.md"
APP = "app.py"
DEVICES = ".claude/settings.json"
SECRET = "AKIA" + "ABCDEFGHIJKLMNOP"  # 이 파일 자체가 비밀 값 검사에 걸리지 않게 나눠 적는다.
SAMPLE = {
    APP: '''"""조립."""

from blocks.sample.code import shout


def main(argv):
    print(shout(" ".join(argv[1:])))
    return 0
''',
    "tests/test_app.py": '''import contextlib
import io
import unittest

import app


class AppTest(unittest.TestCase):
    def test_전체가_돈다(self):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            self.assertEqual(app.main(["app", "hi"]), 0)
        self.assertEqual(out.getvalue(), "HI\\n")
''',
    CARD: """# sample

## 하는 일
글자를 대문자로 바꾼다.

## 입력
- 글자

## 출력
- 대문자 글자

## 금지 사항
- [N1] 빈 글자는 받지 않는다.
- [N2] 줄바꿈이 든 글자는 받지 않는다.
""",
    BLOCK: '''def shout(text):
    if not text:
        raise ValueError("빈 글자")
    if "\\n" in text:
        raise ValueError("줄바꿈")
    return text.upper()
''',
    TEST: '''import unittest

from .code import shout


class ShoutTest(unittest.TestCase):
    def test_대문자로_바꾼다(self):
        self.assertEqual(shout("hi"), "HI")

    def test_N1_빈_글자는_받지_않는다(self):
        with self.assertRaises(ValueError):
            shout("")

    def test_N2_줄바꿈이_든_글자는_받지_않는다(self):
        with self.assertRaises(ValueError):
            shout("a\\nb")
''',
}
UNIT_CARD = "# {name}\n\n## 하는 일\n값을 하나 돌려준다.\n\n## 입력\n- 없음\n\n## 출력\n- 글자\n\n## 금지 사항\n- [N1] 빈 값을 돌려주지 않는다.\n"
UNIT_TEST = (
    "import unittest\n\nfrom .code import run\n\n\nclass RunTest(unittest.TestCase):\n"
    "    def test_N1_빈_값을_돌려주지_않는다(self):\n        self.assertTrue(run())\n"
)
# 확인을 도우미에 맡기고, 비동기로 돌고, 줄바꿈 없이 화면에 글자와 깨진 바이트를 남기고, 임시 폴더로 옮겨 가서 일하는 시험.
# 모두 규칙을 지킨 모양이다.
UNUSUAL_TEST = '''import os
import sys
import tempfile
import unittest

from .code import run


class RunTest(unittest.IsolatedAsyncioTestCase):
    def holds(self, value):
        self.assertTrue(value)

    async def test_N1_빈_값을_돌려주지_않는다(self):
        sys.stdout.write("진행 중...")
        sys.stdout.flush()
        sys.stdout.buffer.write(b"\\xff")
        os.chdir(tempfile.gettempdir())
        from . import late

        self.holds(run() and late.VALUE)
'''
PLAIN_CODE = 'def run():\n    return "ok"\n'
CALLS_BLOCK = 'from blocks.sample.code import shout\n\n\ndef run():\n    return shout("hi")\n'
CALLS_DOOR = "from doors.home.code import run as inner\n\n\ndef run():\n    return inner()\n"
DOOR_CODE = 'import os\n\n\ndef run():\n    return os.environ.get("HOME", "home")\n'
# 프로젝트가 예외를 더해도 이 시험의 뜻이 바뀌지 않게, 임시 프로젝트의 점검 설정은 여기서 고정한다.
SETTINGS = '[tool.blockkit]\nextra-imports = []\n\n[tool.blockkit.pattern-exceptions]\n"check" = ["S603"]\n'


class CheckTest(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.project = Path(temp.name) / "project"
        self.project.mkdir()
        for name in ("check", "uv.lock"):
            shutil.copy(ROOT / name, self.project / name)
        self.write(DEVICES, (ROOT / DEVICES).read_text(encoding="utf-8"))
        project_part = (ROOT / "pyproject.toml").read_text(encoding="utf-8").split("[tool.blockkit]")[0]
        self.write("pyproject.toml", project_part + SETTINGS)
        for name, text in SAMPLE.items():
            self.write(name, text)

    def write(self, name, text):
        path = self.project / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")

    def edit(self, name, old, new):
        text = (self.project / name).read_text(encoding="utf-8")
        self.assertIn(old, text)  # 바꿀 자리가 없으면 위반이 심기지 않은 것이다.
        self.write(name, text.replace(old, new))

    def add_unit(self, kind, name, code, test=UNIT_TEST):
        self.write(f"{kind}/{name}/CARD.md", UNIT_CARD.format(name=name))
        self.write(f"{kind}/{name}/code.py", code)
        self.write(f"{kind}/{name}/test_code.py", test)

    def check(self, *args, stdin="", python=sys.executable, online=False, shared_env=True, **extra_env):
        env = dict(os.environ)
        if shared_env:
            env["UV_PROJECT_ENVIRONMENT"] = str(ROOT / ".venv")  # 임시 프로젝트마다 가상 환경을 새로 만들지 않는다.
        if not online:
            env["UV_OFFLINE"] = "1"  # 인터넷은 의존성 관문만 쓴다. 그 관문을 보는 시험만 인터넷을 켠다.
        env.update(extra_env)
        command = [python, str(self.project / "check"), *args]
        done = subprocess.run(command, input=stdin, env=env, capture_output=True, text=True, encoding="utf-8")
        return done.returncode, done.stdout

    def assert_passes(self):
        code, output = self.check()
        self.assertNotIn("❌", output)
        self.assertEqual(code, 2, output)  # 인터넷을 꺼 두었으므로 의존성만 「점검 못 함」으로 남는다.
        return output

    def assert_fails(self, gate, *expected):
        code, output = self.check()
        self.assertEqual(code, 1, output)
        self.assertIn("❌ " + gate, output)
        for text in expected:
            self.assertIn(text, output)
        return output

    def hook(self, stdin="{}"):
        code, output = self.check("--hook", stdin=stdin)
        self.assertEqual(code, 0, output)  # 훅은 무슨 일이 있어도 0 으로 끝나야 한다.
        return json.loads(output)

    # ---- 기준: 규칙을 지킨 프로젝트는 실패가 없다 ----

    def test_깨끗한_프로젝트는_실패가_없다(self):
        code, output = self.check(online=True)
        self.assertNotIn("❌", output)
        self.assertIn(code, (0, 2), output)  # 2 는 인터넷이 없어서 의존성을 점검하지 못한 경우다.
        for gate in ("카드", "시험", "금지 연결", "통로", "패턴", "비밀 값", "장치"):
            self.assertIn("✅ " + gate, output)
        self.assertIn("4개 통과", output)

    def test_블록과_통로를_더해도_실패가_없다(self):
        self.add_unit("blocks", "second", PLAIN_CODE)
        self.add_unit("doors", "home", DOOR_CODE)
        output = self.assert_passes()
        self.assertIn("블록 2개", output)
        self.assertIn("통로 1개", output)
        self.assertIn("6개 통과", output)

    def test_규칙을_지킨_흔한_모양은_실패로_보지_않는다(self):
        self.write("blocks/sample/rules.py", "def clean(text):\n    return text.strip()\n")
        self.edit(BLOCK, "def shout", "from __future__ import annotations\n\nfrom blocks.sample.rules import clean\n\n\ndef shout")
        self.edit(BLOCK, "return text.upper()", "return clean(text).upper()")
        self.write("blocks/sample/sub/deep.py", "from ..rules import clean\n\n\ndef tidy(text):\n    return clean(text)\n")
        self.add_unit("doors", "wait", DOOR_CODE, UNUSUAL_TEST)
        self.write("doors/wait/late.py", "VALUE = 1\n")
        self.write("blocks/__init__.py", "")
        self.write("blocks/__pycache__/__init__.cpython-313.pyc", "")  # 파이썬이 남기는 캐시
        # 글자 파일이 아닌 파일은 비밀 값 검사에서 뺀다. 캐시에 든 글자 때문에 실패가 나지 않게 하려는 것이다.
        (self.project / "blocks/sample/table.bin").write_bytes(b"\0" + SECRET.encode())
        self.write(".claude/hooks/helper.py", "VALUE = 1\n")  # 숨김 폴더의 코드는 자리 검사에서 뺀다.
        # Claude Code 가 .claude/worktrees/ 에 만드는 작업 사본은 보지 않는다. 사본은 그 안의 check 가 점검한다.
        self.write(".claude/worktrees/fix/tests/test_check.py", 'import subprocess\n\nsubprocess.run(["true"])\n')
        self.write(".claude/worktrees/fix/.venv/lib/tool.py", "def run(value):\n    assert value\n")
        self.write(".claude/worktrees/fix/notes.txt", SECRET + "\n")
        self.assertIn("5개 통과", self.assert_passes())

    def test_맥_기본_파이썬으로_불러도_돈다(self):
        if not os.path.exists("/usr/bin/python3"):
            self.skipTest("맥 기본 파이썬이 없다")
        _, output = self.check(python="/usr/bin/python3")
        self.assertIn("결과:", output)
        self.assertNotIn("❌", output)
        code, reply = self.check("--hook", stdin="{}", python="/usr/bin/python3")
        self.assertEqual(code, 0)
        self.assertIn("결과:", json.loads(reply)["systemMessage"])

    # ---- 카드 ----

    def test_카드가_없으면_실패한다(self):
        (self.project / CARD).unlink()
        self.assert_fails("카드", "CARD.md 가 없습니다")

    def test_카드에_금지_사항이_없으면_실패한다(self):
        self.edit(CARD, "## 금지 사항", "## 메모")
        self.assert_fails("카드", "금지 사항")

    def test_번호가_없는_금지_사항은_실패한다(self):
        self.edit(CARD, "- [N2] 줄바꿈", "- 남의 글자는 받지 않는다.\n- [N2] 줄바꿈")
        self.assert_fails("카드", "남의 글자는")

    def test_번호가_겹친_금지_사항은_실패한다(self):
        self.edit(CARD, "- [N2] 줄바꿈", "- [N1] 욕설은 받지 않는다.\n- [N2] 줄바꿈")
        self.assert_fails("카드", "[N1]", "겹칩니다")

    def test_금지_사항_제목이_두_번_나와도_앞의_약속이_사라지지_않는다(self):
        old = "- [N2] 줄바꿈이 든 글자는 받지 않는다.\n"
        self.edit(CARD, old, old + "\n## 금지 사항\n- [N1] 빈 글자는 받지 않는다.\n")
        self.assert_fails("카드", "[N1]", "겹칩니다")

    def test_정해진_자리_밖의_코드는_실패한다(self):
        for name in ("stray.py", "tests/helpers.py", "blocks/loose.py"):
            self.write(name, "VALUE = 1\n")
        self.assert_fails("카드", "stray.py:", "tests/helpers.py:", "blocks/loose.py:")

    def test_이름이_틀린_시험_파일은_실패한다(self):
        self.write("blocks/sample/code_test.py", UNIT_TEST)
        self.assert_fails("카드", "code_test.py")

    def test_조립_시험이_없으면_실패한다(self):
        (self.project / "tests/test_app.py").unlink()
        self.assert_fails("카드", "tests/test_app.py")

    def test_카드를_읽지_못해도_통과로_보지_않는다(self):
        (self.project / CARD).write_bytes(SAMPLE[CARD].encode("cp949"))
        self.assert_fails("카드")

    # ---- 시험 ----

    def test_블록이_약속을_어기면_깨진_약속의_이름이_나온다(self):
        self.edit(BLOCK, "if not text:", "if text is None:")
        self.edit(BLOCK, "return text.upper()", "return text.lower()")
        self.assert_fails("시험", "blocks/sample: test_N1_빈_글자는", "조립(app.py): test_전체가_돈다")

    def test_건너뛴_시험은_실패한다(self):
        self.edit(TEST, "    def test_N1_", '    @unittest.skip("나중에")\n    def test_N1_')
        self.assert_fails("시험", "건너뜀")

    def test_시험이_하나도_돌지_않는_시험_파일은_실패한다(self):
        self.write("tests/test_app.py", '"""나중에 쓴다."""\n')
        self.assert_fails("시험", "tests/test_app.py", "실행된 시험이 없습니다")

    # ---- 금지 연결 ----

    def test_금지_사항에_시험이_없으면_실패한다(self):
        self.edit(TEST, "def test_N2_", "def test_")
        self.assert_fails("금지 연결", "[N2]")

    def test_확인하는_줄이_없는_시험은_실패한다(self):
        self.edit(TEST, '        with self.assertRaises(ValueError):\n            shout("a\\nb")\n', '        shout("ab")\n')
        self.assert_fails("금지 연결", "[N2]", "확인하는 줄")

    def test_확인이_아닌_호출만_있는_시험은_실패한다(self):
        old = '        with self.assertRaises(ValueError):\n            shout("a\\nb")\n'
        self.edit(TEST, old, '        with self.subTest(text="ab"):\n            shout("ab")\n')
        self.assert_fails("금지 연결", "[N2]", "확인하는 줄")

    def test_실행되지_않는_시험은_있는_것으로_치지_않는다(self):
        self.edit(TEST, "    def test_N1_", "class Parked:\n    def test_N1_")
        self.assert_fails("금지 연결", "[N1]", "실행되지 않았습니다")

    # ---- 통로 ----

    def test_블록이_바깥_모듈을_부르면_실패한다(self):
        self.edit(BLOCK, "def shout", "import socket\n\n\ndef shout")
        self.assert_fails("통로", "socket")

    def test_블록이_파일_화면_시계를_직접_건드리면_실패한다(self):
        self.edit(BLOCK, "def shout", "import datetime\n\n\ndef shout")
        self.edit(BLOCK, "    return text.upper()", '    open("memo.txt")\n    print(datetime.datetime.now())\n    return text.upper()')
        self.assert_fails("통로", "open()", "print()", "now()")

    def test_블록과_통로는_서로를_부르지_못한다(self):
        self.add_unit("doors", "home", DOOR_CODE)
        self.add_unit("blocks", "second", CALLS_BLOCK)
        self.add_unit("doors", "third", CALLS_BLOCK)
        self.add_unit("doors", "fourth", CALLS_DOOR)
        self.assert_fails("통로", "blocks/second/code.py", "doors/third/code.py", "doors/fourth/code.py")

    def test_조립_파일에_판단이_있으면_실패한다(self):
        old = '    print(shout(" ".join(argv[1:])))\n'
        new = '    words = [word for word in argv[1:] if word]\n    for _ in range(1):\n        print(shout(" ".join(words)))\n'
        self.edit(APP, old, new)
        self.assert_fails("통로", "app.py:7", "app.py:8")

    def test_조립_파일이_바깥을_직접_건드리면_실패한다(self):
        self.edit(APP, "from blocks", "import os\n\nfrom blocks")
        self.edit(APP, '" ".join(argv[1:])', 'os.environ.get("NAME", "x")')
        self.assert_fails("통로", "app.py:3", "os")

    # ---- 패턴, 비밀 값 ----

    def test_위험한_패턴은_주석으로_끌_수_없다(self):
        new = "    assert text is not None  # noqa: S101\n    # ruff: disable[S101]\n    assert text != 0\n    return text.upper()"
        self.edit(BLOCK, "    return text.upper()", new)
        self.assert_fails("패턴", "code.py:6:5: S101", "code.py:8:5: S101")

    def test_위험한_패턴은_설정_파일로_끌_수_없다(self):
        self.edit(BLOCK, "    return text.upper()", "    assert text is not None\n    return text.upper()")
        # 규칙 고르기는 check 의 명령이 이기지만, 검사 대상에서 빼기와 파일별 예외는 설정 파일이 끼어들 수 있는 자리다.
        self.write("ruff.toml", 'extend-exclude = ["blocks"]\n\n[lint.extend-per-file-ignores]\n"*" = ["S101"]\n')
        self.assert_fails("패턴", "S101")

    def test_폴더_이름_때문에_패턴_검사에서_빠지지_않는다(self):
        self.add_unit("blocks", "dist", 'def run():\n    assert True\n    return "ok"\n')
        self.assert_fails("패턴", "blocks/dist/code.py")

    def test_패턴_예외는_있는_파일_하나에_규칙_번호_하나씩만_적을_수_있다(self):
        self.edit(BLOCK, "    return text.upper()", "    assert text is not None\n    return text.upper()")
        original = (self.project / "pyproject.toml").read_text(encoding="utf-8")
        old = '"check" = ["S603"]'
        self.assertIn(old, original)
        self.write("*", "")  # 이름이 무늬처럼 생긴 파일이 실제로 있어도 무늬는 받지 않는다.
        # 쉼표로 다른 파일의 예외를 끼워 넣기, 모든 파일, 규칙 묶음 전체, 폴더 무늬, 어느 폴더에나 맞는 파일 이름
        widened = ('"check" = ["S603,*:S101"]', '"*" = ["S101"]', '"blocks/sample/code.py" = ["S"]', '"blocks/**" = ["S101"]', '"code.py" = ["S101"]')
        for new in (widened[0],) + tuple(old + "\n" + line for line in widened[1:]):
            with self.subTest(new=new):
                self.write("pyproject.toml", original.replace(old, new))
                self.assert_fails("패턴", "pattern-exceptions")
        self.write("pyproject.toml", original.replace(old, old + '\n"blocks/sample/code.py" = ["S101"]'))
        self.assertIn("끈 패턴 검사 2개", self.assert_passes())

    def test_패턴_예외는_적은_파일_하나에만_적용된다(self):
        self.add_unit("doors", "web", PLAIN_CODE)
        self.write("doors/web/app.py", "def run(value):\n    assert value\n")  # 맨 위의 app.py 와 이름만 같은 파일
        self.edit("pyproject.toml", '"check" = ["S603"]', '"check" = ["S603"]\n"app.py" = ["S101"]')
        self.assert_fails("패턴", "doors/web/app.py:2:5: S101")

    def test_프로젝트에_놓인_파일이_점검_도구를_가로채지_못한다(self):
        self.edit(BLOCK, "    return text.upper()", "    assert text is not None\n    return text.upper()")
        self.write("ruff.py", "import sys\n\nsys.exit(0)\n")
        self.write("-x.py", "assert True\n")  # 이름이 옵션처럼 생긴 파일도 검사 대상으로만 읽는다.
        self.assert_fails("패턴", "blocks/sample/code.py:6:5: S101", "-x.py:1:1: S101")

    def test_비밀_값이_있으면_실패하고_값은_출력하지_않는다(self):
        self.write("notes.txt", "키: " + SECRET + "\n")
        output = self.assert_fails("비밀 값", "notes.txt:1")
        self.assertNotIn(SECRET, output)

    def test_큰_파일에_든_비밀_값도_찾는다(self):
        self.write("data.txt", "x" * 1_100_000 + "\n" + SECRET + "\n")
        self.assert_fails("비밀 값", "data.txt:2")

    # ---- 숨기기와 꾸미기 ----

    def test_건너뛰는_폴더_이름으로_코드를_숨길_수_없다(self):
        self.write("tools/node_modules/steal.py", "VALUE = 1\n")
        self.write("blocks/__pycache__/code.py", "VALUE = 1\n")
        self.write("blocks/sample/.venv/extra.py", "def run(value):\n    assert value\n")
        self.write("notes/.git/key.txt", SECRET + "\n")
        self.write("notes/uv.lock", SECRET + "\n")
        self.write("notes/.claude/worktrees/key.txt", SECRET + "\n")
        self.add_unit("blocks", "node_modules", "import socket\n\n\ndef run():\n    return socket.gethostname()\n")
        for name in ("blocks/sample/fast.so", "helper.pyc"):  # 읽을 수 없는 코드 파일
            (self.project / name).write_bytes(b"\0")
        output = self.assert_fails("카드", "tools/node_modules/steal.py:", "blocks/__pycache__/code.py:", "blocks/sample/fast.so:", "helper.pyc:")
        self.assertIn("블록 2개", output)
        self.assertIn("❌ 통로", output)
        self.assertIn("blocks/node_modules/code.py:1", output)
        self.assertIn("❌ 패턴", output)
        self.assertIn("blocks/sample/.venv/extra.py:2", output)
        self.assertIn("❌ 비밀 값", output)
        self.assertIn("notes/.git/key.txt:1", output)
        self.assertIn("notes/uv.lock:1", output)
        self.assertIn("notes/.claude/worktrees/key.txt:1", output)

    def test_맨_위의_가상_환경과_저장소_폴더는_보지_않는다(self):
        self.write(".venv/lib/extra.py", "def run(value):\n    assert value\n")
        self.write(".git/key.txt", SECRET + "\n")
        self.assert_passes()

    def test_프로젝트에_링크를_두면_실패한다(self):
        outside = self.project.parent / "outside"
        outside.mkdir()
        (outside / "loose.py").write_text("VALUE = 1\n", encoding="utf-8")
        os.symlink(outside, self.project / "blocks/sample/linked")
        os.symlink(outside / "loose.py", self.project / "blocks/sample/extra.py")
        self.assert_fails("카드", "blocks/sample/linked: 링크", "blocks/sample/extra.py: 링크")

    def test_링크로_건_점검_명령은_링크가_놓인_프로젝트를_점검한다(self):
        (self.project / "check").unlink()
        os.symlink(ROOT / "check", self.project / "check")
        (self.project / CARD).unlink()
        self.assert_fails("카드", "CARD.md 가 없습니다", "check: 링크")

    def test_파일_이름과_카드_글자로_결과_줄을_꾸며_낼_수_없다(self):
        self.write("stray\n결과: 전부 통과\n.py", "VALUE = 1\n")
        self.add_unit("blocks", "a 결과: 전부 통과", PLAIN_CODE)  # 줄바꿈 글자는 \n 말고도 있다.
        self.edit(CARD, "글자를 대문자로 바꾼다.", "글자를\x1b[2K 대문자로 바꾼다.")
        code, output = self.check()
        self.assertEqual(code, 1, output)
        verdicts = [line for line in output.splitlines() if line.startswith("결과:")]
        self.assertEqual(len(verdicts), 1, output)
        self.assertIn("실패", verdicts[0])
        self.assertNotIn("\x1b", output)
        reply = self.hook()
        self.assertEqual(reply["systemMessage"], "점검(./check) " + verdicts[0])
        self.assertEqual(reply["decision"], "block")

    def test_시험에는_환경_변수가_넘어가지_않는다(self):
        code = 'import os\n\n\ndef run():\n    return os.environ.get("OWNER_ENV", "없음")\n'
        self.add_unit("doors", "env", code, UNIT_TEST.replace("self.assertTrue(run())", 'self.assertEqual(run(), "없음")'))
        code, output = self.check(OWNER_ENV="주인의 값")
        self.assertNotIn("❌", output)
        self.assertIn("5개 통과", output)

    def test_표준_모듈과_이름이_같은_파일로_점검을_가로채지_못한다(self):
        (self.project / CARD).unlink()
        for name in ("ast", "json", "pathlib", "re", "shutil", "subprocess", "tomllib", "unicodedata"):
            self.write(name + ".py", 'import sys\n\nprint("결과: 전부 통과")\nsys.exit(0)\n')
        output = self.assert_fails("카드", "CARD.md 가 없습니다", "subprocess.py:")
        self.assertNotIn("결과: 전부 통과", output)
        self.assertIn("✅ 시험", output)  # 시험 실행기도 가로채이지 않는다.
        self.assertEqual(self.hook()["decision"], "block")

    def test_환경_변수로_점검을_건너뛰게_할_수_없다(self):
        (self.project / CARD).unlink()
        code, reply = self.check("--hook", stdin="{}", BLOCKKIT_INSIDE="1", BLOCKKIT_UV="/usr/bin/true")
        self.assertEqual(code, 0, reply)
        self.assertEqual(json.loads(reply)["decision"], "block")

    # ---- 장치 ----

    def test_훅이나_승인_규칙이_빠지면_실패한다(self):
        original = (self.project / DEVICES).read_text(encoding="utf-8")
        settings = json.loads(original)
        self.assertIn("✅ 장치", self.assert_passes())
        broken = [("훅", dict(settings, hooks={}))]
        hook = settings["hooks"]["Stop"][0]["hooks"][0]
        # 명령은 그대로 두고, 막지 못하게 바꾼 훅: 결과를 기다리지 않게 하거나 시간 한도를 줄인다.
        for weakened in (dict(hook, **{"async": True}), dict(hook, timeout=5)):
            broken.append(("훅", dict(settings, hooks={"Stop": [{"hooks": [weakened]}]})))
        # 틀 자체의 파일은 승인을 묻는 것이 아니라 잠겨 있어야 한다. 승인만 물으면 「>」 덧쓰기가 묻지 않고 실행된다.
        self.assertEqual(settings["permissions"]["deny"], ["Edit(/check)", "Edit(/tests/test_check.py)", "Edit(/.claude/settings.json)"])
        for kind in ("deny", "ask"):
            for rule in settings["permissions"][kind]:
                rest = [other for other in settings["permissions"][kind] if other != rule]
                broken.append((rule, dict(settings, permissions=dict(settings["permissions"], **{kind: rest}))))
        # 형식이 틀린 값이 하나라도 섞이면 Claude Code 가 이 파일을 통째로 읽지 않아서, 훅과 승인 규칙이 적혀 있어도 돌지 않는다.
        other_hook = [{"matcher": ["Edit", "Write"], "hooks": [{"type": "command", "command": "true"}]}]
        broken.append(("다른 설정", dict(settings, hooks=dict(settings["hooks"], PreToolUse=other_hook))))
        broken.append(("다른 설정", dict(settings, cleanupPeriodDays="many")))
        for expected, changed in broken:
            with self.subTest(expected=expected):
                self.write(DEVICES, json.dumps(changed))
                self.assert_fails("장치", ".claude/settings.json: 틀이 정한 내용과 다릅니다.")
        (self.project / DEVICES).unlink()
        self.assert_fails("장치")

    def test_훅을_끄는_설정이_있으면_실패한다(self):
        self.write(".claude/settings.local.json", '{"permissions": {"allow": ["Bash(uv run:*)"]}}')
        self.assert_passes()
        self.write(".claude/settings.local.json", '{"disableAllHooks": true}')
        self.assert_fails("장치", "settings.local.json", "disableAllHooks")

    # ---- 의존성 ----

    def test_인터넷이_없으면_의존성은_점검_못_함으로_나온다(self):
        code, output = self.check()
        self.assertEqual(code, 2, output)
        self.assertIn("⚠️ 의존성", output)
        self.assertIn("인터넷", output)

    def test_취약한_외부_부품은_실패하지만_훅은_막지_않는다(self):
        uv = shutil.which("uv")
        added = uv and subprocess.run(
            [uv, "add", "--no-sync", "--project", str(self.project), "urllib3==1.24.1"], capture_output=True, text=True
        )
        if not added or added.returncode:
            self.skipTest("인터넷이 없어서 취약한 부품을 넣어 볼 수 없다")
        # 취약한 부품이 원래 가상 환경에 깔리지 않게 가상 환경을 따로 만든다.
        code, output = self.check(online=True, shared_env=False)
        self.assertEqual(code, 1, output)
        self.assertIn("❌ 의존성", output)
        self.assertIn("urllib3", output)
        code, reply = self.check("--hook", stdin="{}", online=True, shared_env=False)
        self.assertEqual(code, 0)
        self.assertNotIn("decision", json.loads(reply))
        self.assertIn("실패", json.loads(reply)["systemMessage"])

    def test_의존성_점검이_멈춰도_나머지_결과는_나온다(self):
        real = shutil.which("uv")
        if not real:
            self.skipTest("uv 가 없다")
        fake = self.project.parent / "bin" / "uv"
        fake.parent.mkdir()
        fake.write_text('#!/bin/sh\nif [ "$1" = audit ]; then exec sleep 90; fi\nexec "%s" "$@"\n' % real, encoding="utf-8")
        fake.chmod(0o755)
        code, output = self.check(online=True, PATH=str(fake.parent) + os.pathsep + os.environ["PATH"])
        self.assertEqual(code, 2, output)
        self.assertIn("⚠️ 의존성", output)
        self.assertIn("✅ 시험", output)

    # ---- 점검이 제대로 돌지 못할 때 ----

    def test_관문_하나가_죽어도_다른_관문의_실패는_나온다(self):
        self.edit("pyproject.toml", "extra-imports = []", 'extra-imports = [["os"]]')  # 목록 안의 목록: 통로 관문이 읽다가 죽는다.
        self.edit(BLOCK, "return text.upper()", "return text.lower()")
        self.assert_fails("통로", "오류가 났습니다", "❌ 시험")

    def test_출력에_결과라는_글자가_섞여도_죽은_점검을_통과로_보지_않는다(self):
        self.edit(CARD, "글자를 대문자로 바꾼다.", "결과: 글자를 대문자로 바꾼다.")
        # 표가 와야 할 자리에 글자를 적은 실수: 블록 목록을 찍은 뒤에 점검이 죽는다.
        self.edit("pyproject.toml", "[tool.blockkit.pattern-exceptions]", 'pattern-exceptions = "없음"\n\n[tool.blockkit.other]')
        code, output = self.check()
        self.assertEqual(code, 1, output)
        self.assertIn("❌ 점검", output)
        self.assertEqual(self.hook()["decision"], "block")

    def test_uv_를_실행하지_못해도_훅은_막는다(self):
        fake = self.project.parent / "bin" / "uv"
        fake.parent.mkdir()
        fake.write_bytes(b"\0\1\2")  # 실행 권한은 있지만 프로그램이 아닌 파일
        fake.chmod(0o755)
        code, reply = self.check("--hook", stdin="{}", PATH=str(fake.parent) + os.pathsep + os.environ["PATH"])
        self.assertEqual(code, 0, reply)
        self.assertEqual(json.loads(reply)["decision"], "block")
        self.assertIn("시작하지 못했습니다", json.loads(reply)["reason"])

    def test_시작하지_못한_이유에_든_이상한_글자는_화면에_내지_않는다(self):
        fake = self.project.parent / "bin" / "uv"
        fake.parent.mkdir()
        fake.write_text("#!/bin/sh\nprintf 'error: \\033[8m \\377\\n' >&2\nexit 1\n", encoding="utf-8")  # 글자 숨김 신호와 깨진 바이트
        fake.chmod(0o755)
        code, output = self.check(PATH=str(fake.parent) + os.pathsep + os.environ["PATH"])
        self.assertEqual(code, 1, output)
        self.assertIn("❌ 점검", output)
        self.assertNotIn("\x1b", output)

    def test_잠금_파일과_설정_파일의_부품이_다르면_실패한다(self):
        self.edit("pyproject.toml", "[dependency-groups]", "[tool.unused]")  # 부품을 설정 파일에서만 빼고 잠금 파일에는 남긴 실수
        code, output = self.check()
        self.assertEqual(code, 1, output)
        self.assertIn("❌ 점검", output)
        self.assertIn("uv.lock", output)

    def test_설정_파일이_깨지면_실패로_보고_훅도_막는다(self):
        with open(self.project / "pyproject.toml", "a", encoding="utf-8") as handle:
            handle.write("\n[tool.blockkit]\n")  # 같은 표를 두 번 적은 실수
        code, output = self.check()
        self.assertEqual(code, 1, output)
        self.assertIn("❌", output)
        self.assertEqual(self.hook()["decision"], "block")

    # ---- 훅 ----

    def test_훅은_실패가_없으면_한_줄만_알린다(self):
        reply = self.hook()
        self.assertNotIn("decision", reply)
        self.assertIn("결과:", reply["systemMessage"])

    def test_훅은_실패일_때_끝내지_못하게_막는다(self):
        (self.project / CARD).unlink()
        reply = self.hook('{"stop_hook_active": false}')
        self.assertEqual(reply["decision"], "block")
        self.assertIn("❌ 카드", reply["reason"])

    def test_훅은_이미_한_번_막았으면_다시_막지_않고_실패를_알린다(self):
        (self.project / CARD).unlink()
        reply = self.hook('{"stop_hook_active": true}')
        self.assertNotIn("decision", reply)
        self.assertIn("실패", reply["systemMessage"])

    def test_훅은_입력이_이상해도_올바른_답을_내고_실패를_막는다(self):
        (self.project / CARD).unlink()
        for stdin in ("[]", "null", "", '{"stop_hook_active": tr'):
            with self.subTest(stdin=stdin):
                self.assertEqual(self.hook(stdin)["decision"], "block")

    def test_잠금_파일에_없는_부품은_가상_환경에_깔아도_쓸_수_없다(self):
        code, output = self.check(shared_env=False, online=True)  # 임시 프로젝트 안에 가상 환경을 따로 만든다.
        if "❌ 점검" in output:
            self.skipTest("인터넷이 없어서 가상 환경을 새로 만들 수 없다")
        site = next((self.project / ".venv").glob("lib/python*/site-packages"))
        (site / "sneaky.py").write_text('def run():\n    return "ok"\n', encoding="utf-8")
        (site / "sneaky-1.0.dist-info").mkdir()
        (site / "sneaky-1.0.dist-info/METADATA").write_text("Metadata-Version: 2.1\nName: sneaky\nVersion: 1.0\n", encoding="utf-8")
        (site / "sneaky-1.0.dist-info/RECORD").write_text("sneaky.py,,\nsneaky-1.0.dist-info/METADATA,,\nsneaky-1.0.dist-info/RECORD,,\n", encoding="utf-8")
        self.add_unit("doors", "extra", "import sneaky\n\n\ndef run():\n    return sneaky.run()\n")
        code, output = self.check(shared_env=False)
        self.assertEqual(code, 1, output)
        self.assertIn("❌ 시험", output)
        self.assertFalse((site / "sneaky.py").exists())


if __name__ == "__main__":
    unittest.main()
