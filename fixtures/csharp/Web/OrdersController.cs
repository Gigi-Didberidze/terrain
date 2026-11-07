using Acme.Core;
using Microsoft.AspNetCore.Mvc;

namespace Acme.Web
{
    [ApiController]
    public class OrdersController : ControllerBase
    {
        [HttpGet]
        public int Get()
        {
            var order = new Order();
            order.Add(250);
            return order.Total();
        }
    }
}
