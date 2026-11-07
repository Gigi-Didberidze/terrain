module Billing
  module Tax
    RATE = 0.2

    def self.apply(amount)
      (amount * (1 + RATE)).round(2)
    end
  end
end
