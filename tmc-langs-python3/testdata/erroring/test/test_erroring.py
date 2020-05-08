import unittest
from tmc import points


@points('1.1')
class TestErroring(unittest.TestCase):

    @points('1.2', '2.2')
    def test_erroring(self):
        doSomethingIllegal

if __name__ == '__main__':
    unittest.main()
