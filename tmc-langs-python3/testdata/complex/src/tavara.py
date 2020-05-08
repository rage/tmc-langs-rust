class Tavara:

    def __init__(self, nimi, paino):
        self.nimi = nimi
        self.paino = paino

    def __str__(self):
        return "{0} ({1} kg)".format(self.nimi, self.paino)
