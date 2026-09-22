use crate::common;

#[test]
fn constant_return_pop_after_awaited_branches() {
    common::assert_matches_cpython_marshal(
        r#"async def _validate_input(
    user_input: dict[str, Any],
    api: GoogleAirQualityApi,
    errors: dict[str, str],
    description_placeholders: dict[str, str],
) -> bool:
    try:
        custom_options = user_input.get(CUSTOM_LOCAL_AQI_OPTIONS) or {}
        enable_custom_laqi = custom_options.get(CONF_ENABLE_CUSTOM_LAQI)

        if enable_custom_laqi:
            country = custom_options.get(CONF_COUNTRY)
            custom_laqi = custom_options.get(CUSTOM_LAQI)

            # When custom LAQI is enabled, both country and custom_laqi must be provided
            if not country or not custom_laqi:
                errors[CUSTOM_LOCAL_AQI_OPTIONS] = "missing_custom_laqi_options"
                return False

            await api.async_get_current_conditions(
                lat=user_input[CONF_LOCATION][CONF_LATITUDE],
                lon=user_input[CONF_LOCATION][CONF_LONGITUDE],
                region_code=country,
                custom_local_aqi=custom_laqi,
            )
        else:
            await api.async_get_current_conditions(
                lat=user_input[CONF_LOCATION][CONF_LATITUDE],
                lon=user_input[CONF_LOCATION][CONF_LONGITUDE],
            )
    except InvalidCustomLAQIConfigurationError:
        errors["base"] = "mismatch_country_and_laqi"
    except GoogleAirQualityApiError as err:
        errors["base"] = "cannot_connect"
        description_placeholders["error_message"] = str(err)
    except Exception:
        _LOGGER.exception("Unexpected exception")
        errors["base"] = "unknown"
    else:
        return True
    return False
"#,
        "constant_return_pop_after_await.py",
    );
}
