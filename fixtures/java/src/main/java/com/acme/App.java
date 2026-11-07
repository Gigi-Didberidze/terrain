package com.acme;

import com.acme.core.Order;

public class App {
    public static void main(String[] args) {
        Order order = new Order();
        order.add(250);
        System.out.println(order.total());
    }
}
