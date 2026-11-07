module Billing
  class Invoice
    include Comparable

    def initialize(items)
      @items = items
    end

    def total
      Tax.apply(@items.sum)
    end

    def <=>(other)
      total <=> other.total
    end
  end
end
