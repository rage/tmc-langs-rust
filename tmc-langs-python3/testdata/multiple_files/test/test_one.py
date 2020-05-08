import unittest
from tmc import points


@points('1.1')
class TestOne(unittest.TestCase):

    @points('1.2')
    def test_new(self):
        self.assertEqual("a", "a")

if __name__ == '__main__':
    unittest.main()
