using System;
using Xunit;
using TestProject;
using TestMyCode.CSharp.API.Attributes;

namespace TestProjectTests
{
    [Points("1")]
    public class ProgramTest
    {
        [Fact]
        [Points("1.1")]
        public void TestReturnsTrue()
        {
            Assert.True(false);
        }

        [Fact]
        [Points("1.1")]
        public void ReturnsNotInput()
        {
            Assert.True(true);
        }

        [Fact]
        [Points("1.2")]
        public void ReturnsString()
        {
            Assert.True(true);
        }

        [Fact]
        public void TestForClassPoint()
        {
            Assert.True(true);
        }

        public void NotAPointTest()
        {
            Assert.True(false);
        }
    }
}
