#include <shop.h>

int round_cents(int cents)
{
    return (cents + 50) / 100 * 100;
}
