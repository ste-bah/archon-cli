package example;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.assertEquals;
class CalculatorTest {
    @Test void passing() { assertEquals(4, Calculator.add(2, 2)); }
    @Test void failing() { assertEquals(5, Calculator.add(2, 2)); }
}
