package order

type Item struct {
	Price int
}

type Order struct {
	Items []Item
}

func (o *Order) Total() int {
	return sum(o.Items)
}

func sum(items []Item) int {
	t := 0
	for _, i := range items {
		t += i.Price
	}
	return t
}
