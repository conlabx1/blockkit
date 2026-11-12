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
# 확인을 도우미에 맡기고, 비동기로 돌고, 줄바꿈 없이 화면에 글자를 남기는 시험. 모두 규칙을 지킨 모양이다.
UNUSUAL_TEST = '''import sys
import unittest

from .code import run


class RunTest(unittest.IsolatedAsyncioTestCase):
    def holds(self, value):
        self.assertTrue(value)

    async def test_N1_빈_값을_돌려주지_않는다(self):
        sys.stdout.write("진행 중...")
        self.holds(run())
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
        env = {key: value for key, value in os.environ.items() if not key.startswith("BLOCKKIT_")}
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
        for gate in ("카드", "시험", "금지 연결", "통로", "패턴", "비밀 값"):
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
        self.write("blocks/__init__.py", "")
        self.write(".claude/worktrees/fix/helpers.py", "VALUE = 1\n")
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

    def test_비밀_값이_있으면_실패하고_값은_출력하지_않는다(self):
        secret = "AKIA" + "ABCDEFGHIJKLMNOP"  # 이 파일 자체가 걸리지 않게 나눠 적는다.
        self.write("notes.txt", "키: " + secret + "\n")
        output = self.assert_fails("비밀 값", "notes.txt:1")
        self.assertNotIn(secret, output)

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

    def test_설정_파일이_훅과_승인_규칙을_담고_있다(self):
        settings = json.loads((ROOT / ".claude/settings.json").read_text(encoding="utf-8"))
        command = settings["hooks"]["Stop"][0]["hooks"][0]["command"]
        self.assertIn("check", command)
        self.assertIn("--hook", command)
        for path in ("/check", "/pyproject.toml", "/.claude/settings.json", "/tests/test_check.py", "/blocks/**/CARD.md", "/doors/**/CARD.md"):
            self.assertIn("Edit(%s)" % path, settings["permissions"]["ask"])


if __name__ == "__main__":
    unittest.main()
