#ifndef SHOP_H
#define SHOP_H

struct order {
    int items[16];
    int count;
};

int order_total(const struct order *o);
int round_cents(int cents);

#endif
