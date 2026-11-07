TAX_RATE = 0.2


def money(value):
    return round(value, 2)


class LineItem:
    def __init__(self, price, qty):
        self.price = price
        self.qty = qty

    def subtotal(self):
        return money(self.price * self.qty)


class Order:
    def __init__(self):
        self.items = []

    def add(self, item: LineItem):
        self.items.append(item)

    def total(self):
        return money(sum(i.subtotal() for i in self.items) * (1 + TAX_RATE))
