import unittest

from .block import greet


class GreetTest(unittest.TestCase):
    def test_이름을_넣으면_인사말이_나온다(self):
        self.assertEqual(greet("하늘"), "하늘님, 안녕하세요.")

    def test_앞뒤_공백은_떼고_인사한다(self):
        self.assertEqual(greet("  하늘 \n"), "하늘님, 안녕하세요.")

    def test_N1_빈_이름으로는_인사말을_만들지_않는다(self):
        for name in ("", "   "):
            with self.subTest(name=name), self.assertRaises(ValueError):
                greet(name)

    def test_N2_줄바꿈이_든_이름은_받지_않는다(self):
        with self.assertRaises(ValueError):
            greet("하\n늘")
