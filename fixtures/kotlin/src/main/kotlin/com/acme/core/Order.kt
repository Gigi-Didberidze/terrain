package com.acme.core

interface Priced {
    fun total(): Int
}

class Order : Priced {
    private val items = mutableListOf<Int>()

    fun add(price: Int) {
        items.add(price)
    }

    override fun total(): Int {
        return roundCents(items.sum())
    }
}

fun roundCents(cents: Int): Int {
    return (cents + 50) / 100 * 100
}
