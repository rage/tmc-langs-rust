import unittest

from tmc import points

from tmc.utils import load, get_stdout
Tavara = load('src.tavara', 'Tavara')
Matkalaukku = load('src.matkalaukku', 'Matkalaukku')
Lastiruuma = load('src.lastiruuma', 'Lastiruuma')


@points('1.3')
class LastiruumaTest(unittest.TestCase):

    def test_can_init(self):
        self.assertTrue(Lastiruuma(2),
                        "Luokalla Lastiruuma tulee olla konstruktori joka ottaa yhden parametrin")

    def test_has_maksimipaino(self):
        self.assertEqual(1,
                         Lastiruuma(1)._Lastiruuma__maksimipaino,
                         "Lastiruumalla tulee olla piilotettu kenttä maksimipaino, " +
                         "johon asetetaan konstruktorille annettu parametri")
        self.assertEqual(2,
                         Lastiruuma(2)._Lastiruuma__maksimipaino,
                         "Lastiruumalla tulee olla piilotettu kenttä maksimipaino, " +
                         "johon asetetaan konstruktorille annettu parametri")

    def test_has_matkalaukut(self):
        self.assertTrue(hasattr(Lastiruuma(2), '_Lastiruuma__matkalaukut'),
                        "Lastiruumalla tulee olla piilotettu kenttä 'matkalaukut'")

    def test_tavarat_empty_at_start(self):
        self.assertEquals(0,
                          len(Lastiruuma(1)._Lastiruuma__matkalaukut),
                          "Uudessa lastiruumassa ei saa olla matkalaukkuja, jollei niitä ole erikseen sinne lisätty")

    def test_can_add_single(self):
        l = Lastiruuma(2)
        m = Matkalaukku(2)
        l.lisaa_matkalaukku(m)

        self.assertEquals(1,
                          len(l._Lastiruuma__matkalaukut),
                          "Lastiruuman listassa matkalaukut tulee olla yksi alkio " +
                          "kun lastiruumaan on lisätty yksi matkalaukku")
        self.assertEqual(m,
                         l._Lastiruuma__matkalaukut[0],
                         "Lastiruumaan lisätyn matkalaukun täytyy olla listassa 'matkalaukut' lisäämisen jälkeen")

    def test_can_add_multiple(self):
        l = Lastiruuma(5)
        m1 = Matkalaukku(1)
        m2 = Matkalaukku(2)
        l.lisaa_matkalaukku(m1)
        l.lisaa_matkalaukku(m2)

        self.assertEquals(2,
                          len(l._Lastiruuma__matkalaukut),
                          "Lastiruuman listassa matkalaukut tulee olla kaksi alkiota " +
                          "kun lastiruumaan on lisätty kaksi matkalaukkua")
        self.assertTrue(m1 in l._Lastiruuma__matkalaukut,
                        "Lastiruumaan lisätyn matkalaukun täytyy olla listassa 'matkalaukut' lisäämisen jälkeen")
        self.assertTrue(m2 in l._Lastiruuma__matkalaukut,
                        "Lastiruumaan lisätyn matkalaukun täytyy olla listassa 'matkalaukut' lisäämisen jälkeen")

    def test_can_not_add_too_heavy(self):
        l = Lastiruuma(1)
        m = Matkalaukku(2)
        t = Tavara("Kivi", 2)
        m.lisaa_tavara(t)
        l.lisaa_matkalaukku(m)

        self.assertEquals(0,
                          len(l._Lastiruuma__matkalaukut),
                          "Lastiruuman lista 'matkalaukut' ei saa kasvaa kun yritetään lisätä liian isoa matkalaukkua")
        self.assertFalse(m in l._Lastiruuma__matkalaukut,
                         "Lastiruuman lista 'matkalaukut' ei saa sisältää matkalaukkua " +
                         "jota yritettiin lisätä kun yritetään lisätä liian isoa matkalaukkua")

    def test_yhteispaino_init(self):
        self.assertEqual(0,
                         Lastiruuma(1).yhteispaino(),
                         "Matkalaukun yhteispainon tulee olla 0 kg kun yhtään matkalaukkua ei ole lisätty")

    def test_yhteispaino_single(self):
        l = Lastiruuma(15)
        m = Matkalaukku(15)
        m.lisaa_tavara(Tavara("Kivi", 1))
        l.lisaa_matkalaukku(m)

        self.assertEqual(1,
                         l.yhteispaino(),
                         "Lastiruuman yhteispainon tulee olla 1 kg kun siihen on lisätty yksi 1 kg painava matkalaukku")

    def test_yhteispaino_multiple(self):
        l = Lastiruuma(21)

        m = Matkalaukku(10)
        m.lisaa_tavara(Tavara("Kivi", 1))
        m.lisaa_tavara(Tavara("Kivi", 6))
        l.lisaa_matkalaukku(m)

        m2 = Matkalaukku(10)
        m2.lisaa_tavara(Tavara("Kivi", 1))
        m2.lisaa_tavara(Tavara("Kivi", 2))
        l.lisaa_matkalaukku(m2)

        self.assertEqual(10,
                         l.yhteispaino(),
                         "Lastiruuman yhteispainon tulee olla 10 kg kun siihen on lisätty " +
                         "yksi 7 kg painava matkalaukku ja yksi 3 kg painava matkalaukku")

    def test_yhteispaino_failed_add(self):
        m = Matkalaukku(15)
        m.lisaa_tavara(Tavara("Kivi", 15))
        l = Lastiruuma(1)
        l.lisaa_matkalaukku(m)

        self.assertEqual(0,
                         l.yhteispaino(),
                         "Lastiruuman yhteispaino ei saa muuttua kun siihen yritetään lisätä " +
                         "liian painavaa matkalaukkua")

    def test_tulosta_tavarat_empty(self):
        Lastiruuma(1).tulosta_tavarat()
        self.assertEqual(0,
                         len(get_stdout()),
                         "Kutsuttaessa 'tulosta_tavarat' tyhjälle matkalaukulle, ei tule tulostua mitään")

    def test_tulosta_tavarat_multiple(self):
        m = Matkalaukku(10)
        m.lisaa_tavara(Tavara("Kivi", 1))
        m.lisaa_tavara(Tavara("Tiili", 2))
        m2 = Matkalaukku(10)
        m2.lisaa_tavara(Tavara("Vaatteet", 3))
        l = Lastiruuma(10)
        l.lisaa_matkalaukku(m)
        l.lisaa_matkalaukku(m2)
        l.tulosta_tavarat()

        output = get_stdout()
        self.assertTrue("Kivi (1 kg)" in output,
                        "Kutsuttaessa 'tulosta_tavarat' tulee tulostua kaikkien " +
                        "lastiruuman matkalaukujen kaikki tavarat")
        self.assertTrue("Tiili (2 kg)" in output,
                        "Kutsuttaessa 'tulosta_tavarat' tulee tulostua kaikkien " +
                        "lastiruuman matkalaukujen kaikki tavarat")
        self.assertTrue("Vaatteet (3 kg)" in output,
                        "Kutsuttaessa 'tulosta_tavarat' tulee tulostua kaikkien " +
                        "lastiruuman matkalaukujen kaikki tavarat")

    def test_str_empty(self):
        self.assertEqual("0 matkalaukkua (0 kg)",
                         Lastiruuma(1).__str__(),
                         "Tyhjän lastiruuman __str__() metodin tulee palauttaa '0 matkalaukkua (0 kg)'")

    def test_str_single(self):
        m1 = Matkalaukku(5)
        m1.lisaa_tavara(Tavara("Kivi", 1))
        m1.lisaa_tavara(Tavara("Kivi", 2))
        l = Lastiruuma(10)
        l.lisaa_matkalaukku(m1)

        self.assertEqual("1 matkalaukkua (3 kg)",
                         l.__str__(),
                         "Lastiruuman metodin __str__() tulee palauttaa ''1 matkalaukkua (3 kg) " +
                         "kun ruumassa on yksi 3 kg painava matkalaukku")

        def test_str_multiple(self):
            m1 = Matkalaukku(5)
            m1.lisaa_tavara(Tavara("Kivi", 1))
            m1.lisaa_tavara(Tavara("Kivi", 2))
            m2 = Matkalaukku(10)
            m2.lisaa_tavara(Tavara("Kivi", 3))
            m2.lisaa_tavara(Tavara("Kivi", 4))
            l = Lastiruuma(10)
            l.lisaa_matkalaukku(m1)
            l.lisaa_matkalaukku(m2)

            self.assertEqual("2 matkalaukkua (10 kg)",
                             l.__str__(),
                             "Lastiruuman metodin __str__() tulee palauttaa ''2 matkalaukkua (10 kg) " +
                             "kun ruumassa on yksi 3 kg painava matkalaukku ja yksi 7 kg painava matkalaukku")


if __name__ == '__main__':
    unittest.main()
