using System;
using Xunit;
using NonCompilingSample;
using TestMyCode.CSharp.API.Attributes;

namespace PassingSampleTests
{
    [Points("1")]
    public class ProgramTest
    {
        [Fact]
        [Points("1.1")]
        public void TestGetName()
        {
            Assert.Equal("Clare", Program.GetName("Clare"));
        }

        [Fact]
        [Points("1.2")]
        public void TestGetYear()
        {
            Assert.Equal(1900, Program.GetYear(1900));
        }
    }
}
