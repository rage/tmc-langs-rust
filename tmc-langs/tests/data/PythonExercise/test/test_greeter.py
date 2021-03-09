import unittest
from unittest.mock import patch

from tmc import points

from tmc.utils import load, get_stdout
Greeter = load('src.greeter', 'Greeter')


@points('1.0')
class GreeterTest(unittest.TestCase):

    def test_greets(self):
        with patch('builtins.input', side_effect=['Make']) as prompt:
            g = Greeter()
            g.greet()

            output = get_stdout()
            self.assertTrue("Hello Make" in output)

            prompt.assert_called_once_with("Who are you?")
