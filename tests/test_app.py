"""조립된 전체를 실제 진입점으로 돌려 본다. 바깥(네트워크 등)에는 닿지 않는다."""

import contextlib
import io
import unittest

import app


class AppTest(unittest.TestCase):
    def run_app(self, *args):
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = app.main(["app", *args])
        return code, out.getvalue(), err.getvalue()

    def test_이름을_주면_인사말을_내보낸다(self):
        self.assertEqual(self.run_app("하늘"), (0, "하늘님, 안녕하세요.\n", ""))

    def test_이름이_없으면_오류를_알리고_실패로_끝난다(self):
        code, out, err = self.run_app()
        self.assertEqual((code, out), (1, ""))
        self.assertIn("오류", err)
