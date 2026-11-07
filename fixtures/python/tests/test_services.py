from shop.services import checkout


def test_checkout():
    assert checkout([10, 20]) > 0
