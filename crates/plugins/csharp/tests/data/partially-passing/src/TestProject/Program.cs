using System;

namespace TestProject
{
    public class Program
    {
        public static bool ReturnTrue => true;

        public static bool ReturnNotInput(bool input) => !input;
        public static string ReturnInputString(string input) => input;

        public static void Main(string[] args)
        {
			//BEGIN SOLUTION
            Console.WriteLine("Hello Home");
			//END SOLUTION
			//STUB: Console.WriteLine("Stub");
        }
    }
}
