from .tavara import Tavara
from .matkalaukku import Matkalaukku
from .lastiruuma import Lastiruuma

t = Tavara("kivi", 1)
print(t)

m = Matkalaukku(3)
print(m)

m.lisaa_tavara(t)
print(m)

m.lisaa_tavara(Tavara("kivi2", 2))
print(m)

m.lisaa_tavara(Tavara("kivi3", 1))
print(m)

m2 = Matkalaukku(2)
m2.lisaa_tavara(Tavara("kivi4", 1))

m3 = Matkalaukku(10000)
m3.lisaa_tavara(Tavara("norsu", 1000))

r = Lastiruuma(10)
r.lisaa_matkalaukku(m)
r.lisaa_matkalaukku(m2)
r.lisaa_matkalaukku(m3)

print(r)
