import unittest

from tmc import points

from tmc.utils import load, get_stdout
Tavara = load('src.tavara', 'Tavara')
Matkalaukku = load('src.matkalaukku', 'Matkalaukku')


@points('1.2')
class MatkalaukkuTest(unittest.TestCase):

    def test_can_init(self):
        self.assertTrue(Matkalaukku(2),
                        "Luokalla Matkalaukku tulee olla konstruktori joka ottaa yhden parametrin")

    def test_has_maksimipaino(self):
        self.assertEqual(1,
                         Matkalaukku(1)._Matkalaukku__maksimipaino,
                         "Matkalaukulla tulee olla piilotettu kenttä maksimipaino, " +
                         "johon asetetaan konstruktorille annettu parametri")
        self.assertEqual(2,
                         Matkalaukku(2)._Matkalaukku__maksimipaino,
                         "Matkalaukulla tulee olla piilotettu kenttä maksimipaino, " +
                         "johon asetetaan konstruktorille annettu parametri")

    def test_has_tavarat(self):
        self.assertTrue(hasattr(Matkalaukku(2), '_Matkalaukku__tavarat'),
                        "Matkalaukulla tulee olla piilotettu kenttä 'tavarat'")

    def test_tavarat_empty_at_start(self):
        self.assertEquals(0,
                          len(Matkalaukku(1)._Matkalaukku__tavarat),
                          "Uudessa matkalaukussa ei saa olla tavaroita, jollei niitä ole erikseen sinne lisätty")

    def test_can_add_single(self):
        m = Matkalaukku(2)

        t = Tavara("Kivi", 1)
        m.lisaa_tavara(t)

        self.assertEquals(1,
                          len(m._Matkalaukku__tavarat),
                          "Matkalaukun listassa tavarat tulee olla yksi alkio kun matkalaukkuun on lisätty yksi tavara")
        self.assertEqual(t,
                         m._Matkalaukku__tavarat[0],
                         "Matkalaukkuun lisätyn tavaran täytyy olla listassa 'tavarat' lisäämisen jälkeen")

    def test_can_add_multiple(self):
        m = Matkalaukku(5)
        t = Tavara("Kivi", 1)
        m.lisaa_tavara(t)
        t2 = Tavara("Tiili", 2)
        m.lisaa_tavara(t2)

        self.assertEquals(2,
                          len(m._Matkalaukku__tavarat),
                          "Matkalaukun listassa tavarat tulee olla kaksi alkiota " +
                          "kun matkalaukkuun on lisätty kaksi tavaraa")
        self.assertTrue(t in m._Matkalaukku__tavarat,
                        "Matkalaukkuun lisätyn tavaran täytyy olla listassa 'tavarat' lisäämisen jälkeen")
        self.assertTrue(t2 in m._Matkalaukku__tavarat,
                        "Matkalaukkuun lisätyn tavaran täytyy olla listassa 'tavarat' lisäämisen jälkeen")

    def test_can_not_add_too_heavy(self):
        m = Matkalaukku(1)
        t = Tavara("Kivi", 2)
        m.lisaa_tavara(t)

        self.assertEquals(0,
                          len(m._Matkalaukku__tavarat),
                          "Matkalaukun lista 'tavarat' ei saa kasvaa kun yritetään lisätä liian isoa esinettä")
        self.assertFalse(t in m._Matkalaukku__tavarat,
                         "Matkalaukun lista 'tavarat' ei saa sisältää tavaraa " +
                         "jota yritettiin lisätä kun yritetään lisätä liian isoa esinettä")

    def test_yhteispaino_init(self):
        self.assertEqual(0,
                         Matkalaukku(1).yhteispaino(),
                         "Matkalaukun yhteispainon tulee olla 0 kg kun yhtään tavaraa ei ole lisätty")

    def test_yhteispaino_single(self):
        m = Matkalaukku(15)
        m.lisaa_tavara(Tavara("Kivi", 1))

        self.assertEqual(1,
                         m.yhteispaino(),
                         "Matkalaukun yhteispainon tulee olla 1 kg kun siihen on lisätty yksi 1 kg painava tavara")

    def test_yhteispaino_multiple(self):
        m = Matkalaukku(15)
        m.lisaa_tavara(Tavara("Kivi", 1))
        m.lisaa_tavara(Tavara("Kivi", 6))

        self.assertEqual(7,
                         m.yhteispaino(),
                         "Matkalaukun yhteispainon tulee olla 7 kg kun siihen on lisätty " +
                         "yksi 1 kg painava tavara ja yksi 6 kg painava tavara")

    def test_yhteispaino_failed_add(self):
        m = Matkalaukku(1)
        m.lisaa_tavara(Tavara("Kivi", 15))

        self.assertEqual(0,
                         m.yhteispaino(),
                         "Matkalaukun yhteispaino ei saa muuttua kun siihen yritetään lisätä liian painavaa tavaraa")

    def test_raskain_tavara_empty(self):
        self.assertIsNone(Matkalaukku(1).raskain_tavara(),
                          "Metodin 'raskain_tavara' tulee palauttaa 'None' kun yhtään esinettä ei ole vielä lisätty")

    def test_raskain_tavara_single(self):
        t = Tavara("kivi", 1)
        m = Matkalaukku(1)
        m.lisaa_tavara(t)

        self.assertEqual(t,
                         m.raskain_tavara(),
                         "Matkalaukun ainoan tavaran tulee olla sen raskain tavara")

    def test_raskain_tavara_multiple(self):
        m = Matkalaukku(10)
        t = Tavara("kivi", 3)
        m.lisaa_tavara(Tavara("Kivi", 1))
        m.lisaa_tavara(t)
        m.lisaa_tavara(Tavara("Kivi", 1))

        self.assertEqual(t,
                         m.raskain_tavara(),
                         "Matkalaukun metodin 'raskain_tavara' tulee palauttaa raskain tavara")

    def test_tulosta_tavarat_empty(self):
        Matkalaukku(1).tulosta_tavarat()
        self.assertEqual(0,
                         len(get_stdout()),
                         "Kutsuttaessa 'tulosta_tavarat' tyhjälle matkalaukulle, ei tule tulostua mitään")

    def test_tulosta_tavarat_multiple(self):
        m = Matkalaukku(10)
        m.lisaa_tavara(Tavara("Kivi", 1))
        m.lisaa_tavara(Tavara("Tiili", 2))
        m.tulosta_tavarat()

        output = get_stdout()
        self.assertTrue("Kivi (1 kg)" in output,
                        "Kutsuttaessa 'tulosta_tavarat' tulee tulostua kaikki matkalaukun tavarat")
        self.assertTrue("Tiili (2 kg)" in output,
                        "Kutsuttaessa 'tulosta_tavarat' tulee tulostua kaikki matkalaukun tavarat")

    def test_str_empty(self):
        self.assertEqual("ei tavaroita (0 kg)",
                         Matkalaukku(0).__str__(),
                         "Tyhjän matkalaukun __str__() metodin tulee palauttaa 'ei tavaroita (0 kg)")

    def test_str_single(self):
        m = Matkalaukku(1)
        m.lisaa_tavara(Tavara("Kivi", 1))
        self.assertEqual("1 tavara (1 kg)",
                         m.__str__(),
                         "Kun matkalaukussa on yksi 1 kg painoinen esine, tulee matkalaukun " +
                         "__str__() metodin palauttaa '1 tavaraa (1 kg)")

    def test_str_multiple(self):
        m = Matkalaukku(5)
        m.lisaa_tavara(Tavara("Kivi", 1))
        m.lisaa_tavara(Tavara("Kivi", 2))
        self.assertEqual("2 tavaraa (3 kg)",
                         m.__str__(),
                         "Kun matkalaukussa on yksi 1 kg painoinen esine ja yksi 2 kg painoinen esine, " +
                         "tulee matkalaukun __str__() metodin palauttaa '2 tavaraa (3 kg)")

if __name__ == '__main__':
    unittest.main()
