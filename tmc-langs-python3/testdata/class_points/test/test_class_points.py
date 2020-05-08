import unittest
from tmc import points


@points('1.5')
class TestClassPoints(unittest.TestCase):

    def test_class_points(self):
        self.assertEqual("a", "a")

    def test_more_class_points(self):
        self.assertEqual("a", "a")

if __name__ == '__main__':
    unittest.main()
