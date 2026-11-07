package com.acme.core;

import static org.junit.jupiter.api.Assertions.assertEquals;

import org.junit.jupiter.api.Test;

class OrderTest {
    @Test
    void totalRoundsToWholeUnits() {
        Order order = new Order();
        order.add(250);
        assertEquals(300, order.total());
    }
}
