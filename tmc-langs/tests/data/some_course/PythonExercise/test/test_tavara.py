import unittest

from tmc import points

from tmc.utils import load
Tavara = load('src.tavara', 'Tavara')


@points('1.1')
class TavaraTest(unittest.TestCase):

    def test_can_init(self):
        self.assertTrue(Tavara("a", 1),
                        "Luokalla Tavara tulee olla konstruktori joka ottaa kaksi arvoa")

    def test_has_name(self):
        self.assertEqual("Kivi",
                         Tavara("Kivi", 1).nimi,
                         "Tavaralla tulee olla muuttuja nimi johon asetetaan konstruktorin ensimmäinen parametri")
        self.assertEqual("Tiili",
                         Tavara("Tiili", 1).nimi,
                         "Tavaralla tulee olla muuttuja nimi johon asetetaan konstruktorin ensimmäinen parametri")

    def test_has_name(self):
        self.assertEqual(1,
                         Tavara("Kivi", 1).paino,
                         "Tavaralla tulee olla muuttuja paino johon asetetaan konstruktorin toinen parametri")
        self.assertEqual(2,
                         Tavara("Tiili", 2).paino,
                         "Tavaralla tulee olla muuttuja paino johon asetetaan konstruktorin toinen parametri")

    def test_correct_str(self):
        self.assertEqual("Kivi (1 kg)",
                         Tavara("Kivi", 1).__str__(),
                         "__str__() metodisi palauttaa väärän arvon kun tavaran nimi on 'Kivi' ja paino 1 kg")

        self.assertEqual("Tiili (2 kg)",
                         Tavara("Tiili", 2).__str__(),
                         "__str__() metodisi palauttaa väärän arvon kun tavaran nimi on 'Tiili' ja paino 2 kg")

if __name__ == '__main__':
    unittest.main()
