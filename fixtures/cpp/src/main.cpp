#include <iostream>
#include <memory>

#include "geo/shape.hpp"

int main() {
    std::unique_ptr<geo::Shape> s = std::make_unique<geo::Circle>(2.0);
    std::cout << s->area() << "\n";
    return 0;
}
