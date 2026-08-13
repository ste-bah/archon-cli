package com.example.demo;

import static org.junit.jupiter.api.Assertions.assertEquals;

import org.junit.jupiter.api.Test;

/**
 * One passing test and one failing one.
 *
 * <p>The failure is deliberate: it is what proves the JUnit report is read
 * back and surfaced. A fixture where everything passes cannot distinguish
 * "the test stage reported no failures" from "the test stage never ran".
 */
class VulnerableTest {

    @Test
    void tallySumsItsArguments() {
        assertEquals(45, new Vulnerable().tally(1, 2, 3, 4, 5, 6, 7, 8, 9));
    }

    @Test
    void deliberatelyFailsSoTheReportHasSomethingToParse() {
        assertEquals(400, new Vulnerable().tally(1, 1, 1, 1, 1, 1, 1, 1, 1));
    }
}
