using System;
using Xunit;
using FailingSample;
using TestMyCode.CSharp.API.Attributes;

namespace FailingSampleTests
{
    [Points("2")]
    public class ProgramTest
    {
        [Fact]
        [Points("2.1")]
        public void TestCheckSameFailed()
        {
            Assert.False(Program.CheckSame);
        }
    }
}
