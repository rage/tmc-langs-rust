from functools import reduce


class Lastiruuma:

    def __init__(self, maksimipaino):
        self.__maksimipaino = maksimipaino
        self.__matkalaukut = []

    def lisaa_matkalaukku(self, matkalaukku):
        if (self.yhteispaino() + matkalaukku.yhteispaino()) <= self.__maksimipaino:
            self.__matkalaukut.append(matkalaukku)

    def yhteispaino(self):
        if not self.__matkalaukut:
            return 0
        return reduce(lambda x, y: x+y, map(lambda x: x.yhteispaino(), self.__matkalaukut))

    def __str__(self):
        return "{0} matkalaukkua ({1} kg)".format(len(self.__matkalaukut),
                                                  self.yhteispaino())

    def tulosta_tavarat(self):
        for matkalaukku in self.__matkalaukut:
            matkalaukku.tulosta_tavarat()
