namespace Acme.Core
{
    internal static class Money
    {
        public static int Round(int cents)
        {
            return (cents + 50) / 100 * 100;
        }
    }
}
