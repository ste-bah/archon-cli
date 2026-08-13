package com.example.demo;

import java.sql.Connection;
import java.sql.ResultSet;
import java.sql.Statement;

/**
 * Deliberately defective. Every method here trips a specific rule, and the
 * point of the fixture is that the TOOLS name the defect — not that a model
 * reads the code and says something plausible about it.
 *
 * <p>This file is shared by the Gradle and Maven fixture projects so both are
 * analysing identical source, and any difference in findings is a difference
 * between the build systems rather than between the inputs.
 */
public class Vulnerable {

    /**
     * User input concatenated into SQL. FindSecBugs reports
     * SQL_INJECTION_JDBC with cweid="89".
     */
    public ResultSet lookup(Connection connection, String userId) throws Exception {
        Statement statement = connection.createStatement();
        return statement.executeQuery("SELECT * FROM accounts WHERE id = '" + userId + "'");
    }

    /**
     * Swallowed exception. PMD reports EmptyCatchBlock.
     *
     * <p>The variable must not be called {@code ignored} or {@code expected}:
     * EmptyCatchBlock treats those names as a deliberate, documented decision
     * and stays silent, so the fixture would prove nothing.
     */
    public void swallow() {
        try {
            Thread.sleep(1);
        } catch (InterruptedException e) {
        }
    }

    /**
     * Nine parameters. Checkstyle reports ParameterNumber and PMD reports
     * ExcessiveParameterList — the pair Sonar calls java:S107.
     */
    public int tally(int a, int b, int c, int d, int e, int f, int g, int h, int i) {
        return a + b + c + d + e + f + g + h + i;
    }
}
