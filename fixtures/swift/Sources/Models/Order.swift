public protocol Priced {
    func total() -> Int
}

public struct Order: Priced {
    public private(set) var items: [Int] = []

    public init() {}

    public mutating func add(_ price: Int) {
        items.append(price)
    }

    public func total() -> Int {
        return roundCents(items.reduce(0, +))
    }
}

func roundCents(_ cents: Int) -> Int {
    return (cents + 50) / 100 * 100
}
