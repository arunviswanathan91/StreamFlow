package org.streamflow.ui;

import org.junit.jupiter.api.Test;
import org.kordamp.ikonli.fontawesome5.FontAwesomeSolid;
import org.kordamp.ikonli.materialdesign2.MaterialDesignC;

import static org.junit.jupiter.api.Assertions.assertEquals;

/**
 * Pins the Ikonli literals used in the UI to their pack enums, so a typo'd literal can't silently
 * fall back to a glyph at runtime. Referencing the enum constants is itself a compile-time check
 * that the icons exist in FontAwesome5 / MaterialDesign2.
 */
class IconLiteralTest {

    @Test
    void uiIconLiteralsMatchPacks() {
        assertEquals("fas-chevron-left", FontAwesomeSolid.CHEVRON_LEFT.getDescription());
        assertEquals("fas-chevron-right", FontAwesomeSolid.CHEVRON_RIGHT.getDescription());
        assertEquals("fas-copy", FontAwesomeSolid.COPY.getDescription());
        assertEquals("fas-tags", FontAwesomeSolid.TAGS.getDescription());
        assertEquals("mdi2c-cog", MaterialDesignC.COG.getDescription());
    }
}
