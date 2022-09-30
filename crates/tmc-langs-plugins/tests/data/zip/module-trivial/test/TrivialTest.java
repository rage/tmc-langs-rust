
import fi.helsinki.cs.tmc.edutestutils.Points;
import org.junit.Test;
import static org.junit.Assert.*;

@Points("trivial")
public class TrivialTest {
    @Test
    public void testF() {
        Trivial t = new Trivial();
        assertEquals(1, t.f());
    }
}
