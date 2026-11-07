#include "geo/shape.hpp"

namespace {
double square(double x) { return x * x; }
}  // namespace

double geo::Circle::area() const { return 3.14159 * square(r_); }
