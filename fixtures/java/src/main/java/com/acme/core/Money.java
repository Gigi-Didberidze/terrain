package com.acme.core;

final class Money {
    private Money() {}

    static int round(int cents) {
        return Math.round(cents / 100f) * 100;
    }
}
