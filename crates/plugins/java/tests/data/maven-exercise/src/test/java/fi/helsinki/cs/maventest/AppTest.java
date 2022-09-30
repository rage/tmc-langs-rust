package fi.helsinki.cs.maventest;

import fi.helsinki.cs.tmc.edutestutils.MockStdio;
import fi.helsinki.cs.tmc.edutestutils.Points;
import fi.helsinki.cs.tmc.edutestutils.ReflectionUtils;
import org.junit.Test;
import static org.junit.Assert.*;
import org.junit.Rule;

public class AppTest {
    @Rule
    public MockStdio mio = new MockStdio();
    
    @Test
    @Points("maven-exercise")
    public void trol() {
        App.main(null);
        assertEquals("Hello Maven!\n", mio.getSysOut());
        ReflectionUtils.newInstanceOfClass("App");


        System.out.println("Tests executed");
    }
}
