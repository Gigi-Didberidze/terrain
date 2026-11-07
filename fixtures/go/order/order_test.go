package order

import "testing"

func TestTotal(t *testing.T) {
	o := &Order{Items: []Item{{Price: 2}}}
	if o.Total() != 2 {
		t.Fatal("total")
	}
}
