package main

import (
	"fmt"

	"example.com/shop/order"
)

func main() {
	o := order.Order{}
	fmt.Println(o.Total())
}
