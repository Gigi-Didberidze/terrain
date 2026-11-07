package com.acme.core;

import java.util.ArrayList;
import java.util.List;

public class Order implements Priced {
    private final List<Integer> items = new ArrayList<>();

    public void add(int price) {
        items.add(price);
    }

    @Override
    public int total() {
        int sum = 0;
        for (int i : items) {
            sum += i;
        }
        return Money.round(sum);
    }
}
