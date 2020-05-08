import unittest
from tmc import points


@points('2.1')
class TestTwo(unittest.TestCase):

    @points('2.2')
    def test_new(self):
        self.assertEqual("a", "a")

if __name__ == '__main__':
    unittest.main()
