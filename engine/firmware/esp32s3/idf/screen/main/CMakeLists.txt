idf_component_register(SRCS "screen_main.c" "wrap.c"
                       PRIV_REQUIRES knobconsole esp_lcd esp_driver_spi esp_driver_gpio esp_driver_ledc esp_timer mbedtls)
foreach(fn esp_lcd_panel_io_tx_param esp_lcd_panel_io_tx_color esp_lcd_panel_draw_bitmap
           spi_device_queue_trans spi_device_polling_transmit spi_device_transmit)
    target_link_libraries(${COMPONENT_LIB} INTERFACE "-Wl,--wrap=${fn}")
endforeach()
