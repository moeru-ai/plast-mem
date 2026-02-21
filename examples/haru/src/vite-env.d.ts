/* eslint-disable jsdoc/check-param-names */
/* eslint-disable sonarjs/use-type-alias */
/* eslint-disable ts/method-signature-style */
/// <reference types="vite/client" />

// Copied from https://github.com/microsoft/TypeScript/blob/main/src/lib/es2025.intl.d.ts
declare namespace Intl {
  /**
   * The Intl.DurationFormat object enables language-sensitive duration formatting.
   *
   * [MDN](https://developer.mozilla.org/docs/Web/JavaScript/Reference/Global_Objects/Intl/DurationFormat)
   */
  interface DurationFormat {
    /**
     * @param duration The duration object to be formatted. It should include some or all of the following properties: months, weeks, days, hours, minutes, seconds, milliseconds, microseconds, nanoseconds.
     *
     * [MDN](https://developer.mozilla.org/docs/Web/JavaScript/Reference/Global_Objects/Intl/DurationFormat/format).
     */
    format(duration: Partial<Record<DurationFormatUnit, number>>): string
    /**
     * @param duration The duration object to be formatted. It should include some or all of the following properties: months, weeks, days, hours, minutes, seconds, milliseconds, microseconds, nanoseconds.
     *
     * [MDN](https://developer.mozilla.org/docs/Web/JavaScript/Reference/Global_Objects/Intl/DurationFormat/formatToParts).
     */
    formatToParts(duration: Partial<Record<DurationFormatUnit, number>>): DurationFormatPart[]
    /**
     * [MDN](https://developer.mozilla.org/docs/Web/JavaScript/Reference/Global_Objects/Intl/DurationFormat/resolvedOptions).
     */
    resolvedOptions(): ResolvedDurationFormatOptions
  }

  /**
   * Whether to always display a unit, or only if it is non-zero.
   *
   * [MDN](https://developer.mozilla.org/docs/Web/JavaScript/Reference/Global_Objects/Intl/DurationFormat/DurationFormat#display).
   */
  type DurationFormatDisplayOption = 'always' | 'auto'

  /**
   * The locale matching algorithm to use.
   *
   * [MDN](https://developer.mozilla.org/docs/Web/JavaScript/Reference/Global_Objects/Intl#Locale_negotiation).
   */
  type DurationFormatLocaleMatcher = 'best fit' | 'lookup'

  /**
   * An object with some or all properties of the `Intl.DurationFormat` constructor `options` parameter.
   *
   * [MDN](https://developer.mozilla.org/docs/Web/JavaScript/Reference/Global_Objects/Intl/DurationFormat/DurationFormat#parameters)
   */
  interface DurationFormatOptions {
    days?: 'long' | 'narrow' | 'short'
    daysDisplay?: DurationFormatDisplayOption
    fractionalDigits?: 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9
    hours?: '2-digit' | 'long' | 'narrow' | 'numeric' | 'short'
    hoursDisplay?: DurationFormatDisplayOption
    localeMatcher?: DurationFormatLocaleMatcher
    microseconds?: 'long' | 'narrow' | 'numeric' | 'short'
    microsecondsDisplay?: DurationFormatDisplayOption
    milliseconds?: 'long' | 'narrow' | 'numeric' | 'short'
    millisecondsDisplay?: DurationFormatDisplayOption
    minutes?: '2-digit' | 'long' | 'narrow' | 'numeric' | 'short'
    minutesDisplay?: DurationFormatDisplayOption
    months?: 'long' | 'narrow' | 'short'
    monthsDisplay?: DurationFormatDisplayOption
    nanoseconds?: 'long' | 'narrow' | 'numeric' | 'short'
    nanosecondsDisplay?: DurationFormatDisplayOption
    numberingSystem?: string
    seconds?: '2-digit' | 'long' | 'narrow' | 'numeric' | 'short'
    secondsDisplay?: DurationFormatDisplayOption
    style?: DurationFormatStyle
    weeks?: 'long' | 'narrow' | 'short'
    weeksDisplay?: DurationFormatDisplayOption
    years?: 'long' | 'narrow' | 'short'
    yearsDisplay?: DurationFormatDisplayOption
  }

  /**
   * An object representing the relative time format in parts
   * that can be used for custom locale-aware formatting.
   *
   * [MDN](https://developer.mozilla.org/docs/Web/JavaScript/Reference/Global_Objects/Intl/DurationFormat/formatToParts).
   */
  type DurationFormatPart
    = | {
      type: 'literal'
      unit?: DurationFormatUnitSingular
      value: string
    }
    | {
      type: Exclude<NumberFormatPartTypes, 'literal'>
      unit: DurationFormatUnitSingular
      value: string
    }

  /**
   * The style of the formatted duration.
   *
   * [MDN](https://developer.mozilla.org/docs/Web/JavaScript/Reference/Global_Objects/Intl/DurationFormat/DurationFormat#style).
   */
  type DurationFormatStyle = 'digital' | 'long' | 'narrow' | 'short'

  /**
   * Value of the `unit` property in duration objects
   *
   * [MDN](https://developer.mozilla.org/docs/Web/JavaScript/Reference/Global_Objects/Intl/DurationFormat/format#duration).
   */
  type DurationFormatUnit
    = | 'days'
      | 'hours'
      | 'microseconds'
      | 'milliseconds'
      | 'minutes'
      | 'months'
      | 'nanoseconds'
      | 'seconds'
      | 'weeks'
      | 'years'

  type DurationFormatUnitSingular
    = | 'day'
      | 'hour'
      | 'microsecond'
      | 'millisecond'
      | 'minute'
      | 'month'
      | 'nanosecond'
      | 'second'
      | 'week'
      | 'year'

  interface ResolvedDurationFormatOptions {
    days: 'long' | 'narrow' | 'short'
    daysDisplay: DurationFormatDisplayOption
    fractionalDigits?: 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9
    hours: '2-digit' | 'long' | 'narrow' | 'numeric' | 'short'
    hoursDisplay: DurationFormatDisplayOption
    locale: UnicodeBCP47LocaleIdentifier
    microseconds: 'long' | 'narrow' | 'numeric' | 'short'
    microsecondsDisplay: DurationFormatDisplayOption
    milliseconds: 'long' | 'narrow' | 'numeric' | 'short'
    millisecondsDisplay: DurationFormatDisplayOption
    minutes: '2-digit' | 'long' | 'narrow' | 'numeric' | 'short'
    minutesDisplay: DurationFormatDisplayOption
    months: 'long' | 'narrow' | 'short'
    monthsDisplay: DurationFormatDisplayOption
    nanoseconds: 'long' | 'narrow' | 'numeric' | 'short'
    nanosecondsDisplay: DurationFormatDisplayOption
    numberingSystem: string
    seconds: '2-digit' | 'long' | 'narrow' | 'numeric' | 'short'
    secondsDisplay: DurationFormatDisplayOption
    style: DurationFormatStyle
    weeks: 'long' | 'narrow' | 'short'
    weeksDisplay: DurationFormatDisplayOption
    years: 'long' | 'narrow' | 'short'
    yearsDisplay: DurationFormatDisplayOption
  }

  const DurationFormat: {
    prototype: DurationFormat

    /**
     * @param locales A string with a BCP 47 language tag, or an array of such strings.
     *   For the general form and interpretation of the `locales` argument, see the [Intl](https://developer.mozilla.org/docs/Web/JavaScript/Reference/Global_Objects/Intl#locale_identification_and_negotiation)
     *   page.
     *
     * @param options An object for setting up a duration format.
     *
     * [MDN](https://developer.mozilla.org/docs/Web/JavaScript/Reference/Global_Objects/Intl/DurationFormat/DurationFormat).
     */
    new (locales?: LocalesArgument, options?: DurationFormatOptions): DurationFormat

    /**
     * Returns an array containing those of the provided locales that are supported in display names without having to fall back to the runtime's default locale.
     *
     * @param locales A string with a BCP 47 language tag, or an array of such strings.
     *   For the general form and interpretation of the `locales` argument, see the [Intl](https://developer.mozilla.org/docs/Web/JavaScript/Reference/Global_Objects/Intl#locale_identification_and_negotiation)
     *   page.
     *
     * @param options An object with a locale matcher.
     *
     * @returns An array of strings representing a subset of the given locale tags that are supported in display names without having to fall back to the runtime's default locale.
     *
     * [MDN](https://developer.mozilla.org/docs/Web/JavaScript/Reference/Global_Objects/Intl/DurationFormat/supportedLocalesOf).
     */
    supportedLocalesOf(locales?: LocalesArgument, options?: { localeMatcher?: DurationFormatLocaleMatcher }): UnicodeBCP47LocaleIdentifier[]
  }
}
