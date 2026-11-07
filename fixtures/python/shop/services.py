from . import pricing
from .models import LineItem, Order


def checkout(prices):
    order = Order()
    for p in prices:
        order.add(LineItem(p, 1))
    return pricing.apply_discount(order, 10)
