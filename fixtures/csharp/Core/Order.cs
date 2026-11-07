using System.Collections.Generic;

namespace Acme.Core
{
    public class Order : IPriced
    {
        private readonly List<int> items = new List<int>();

        public void Add(int price)
        {
            items.Add(price);
        }

        public int Total()
        {
            var sum = 0;
            foreach (var i in items) sum += i;
            return Money.Round(sum);
        }
    }

    public interface IPriced
    {
        int Total();
    }
}
