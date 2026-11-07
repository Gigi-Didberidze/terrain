#include <stdio.h>
#include <shop.h>

int main(void)
{
    struct order o = { .items = { 250 }, .count = 1 };
    printf("%d\n", order_total(&o));
    return 0;
}
