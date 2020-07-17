using System;

namespace NonCompilingSample
{
    public class Program
    {
        public static string GetName(string name) => name;

        public static int GetYear(int year) => year;

        public static void Main(string[] args)
        {
            Console.WriteLine("This is a passing test.");
            #error This is a compile error
        }
    }
}
