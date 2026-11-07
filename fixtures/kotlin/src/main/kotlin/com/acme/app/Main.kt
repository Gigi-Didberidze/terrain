package com.acme.app

import com.acme.core.Order

fun main() {
    val order = Order()
    order.add(250)
    println(order.total())
}
