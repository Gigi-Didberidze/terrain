#include "../include/shop.h"

static int sum(const int *values, int n)
{
    int total = 0;
    for (int i = 0; i < n; i++)
        total += values[i];
    return total;
}

int order_total(const struct order *o)
{
    return round_cents(sum(o->items, o->count));
}
