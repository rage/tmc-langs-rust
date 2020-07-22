from functools import reduce


class Matkalaukku:

    def __init__(self, maksimipaino):
        self.__maksimipaino = maksimipaino
        self.__tavarat = []

    def __str__(self):
        if not self.__tavarat:
            return "ei tavaroita (0 kg)"
        else:
            if len(self.__tavarat) == 1:
                muoto = "tavara"
            else:
                muoto = "tavaraa"
        return "{0} {1} ({2} kg)".format(len(self.__tavarat), muoto, self.yhteispaino())

    def yhteispaino(self):
        if not self.__tavarat:
            return 0
        return reduce(lambda x, y: x+y, map(lambda x: x.paino, self.__tavarat))

    def lisaa_tavara(self, tavara):
        if self.yhteispaino() + tavara.paino <= self.__maksimipaino:
            self.__tavarat.append(tavara)

    def tulosta_tavarat(self):
        for tavara in self.__tavarat:
            print(tavara)

    def raskain_tavara(self):
        if not self.__tavarat:
            return None
        return reduce(lambda max, t: max if t.paino <= max.paino else t, self.__tavarat)
