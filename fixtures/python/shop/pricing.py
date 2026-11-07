from shop.models import Order, money


def apply_discount(order: Order, percent):
    return money(order.total() * (1 - percent / 100))
