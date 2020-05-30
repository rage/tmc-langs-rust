import unittest
from tmc import points


class TestPoints(unittest.TestCase):

    @points('1.1')
    def test_somepoints(self):
        self.assertEqual("a", "a")

if __name__ == '__main__':
    unittest.main()
