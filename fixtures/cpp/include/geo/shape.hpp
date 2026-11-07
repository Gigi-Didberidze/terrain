#pragma once

namespace geo {

class Shape {
  public:
    virtual ~Shape() = default;
    virtual double area() const = 0;
};

class Circle : public Shape {
  public:
    explicit Circle(double r) : r_(r) {}
    double area() const override;

  private:
    double r_;
};

}  // namespace geo
