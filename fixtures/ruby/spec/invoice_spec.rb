require_relative "../lib/billing"

RSpec.describe Billing::Invoice do
  it "adds tax" do
    expect(Billing::Invoice.new([10]).total).to eq(12.0)
  end
end
