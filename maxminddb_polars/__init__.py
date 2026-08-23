"""Polars expressions for MaxMind DB lookups."""

from importlib.metadata import version

from maxminddb_polars._api import lookup_path

__version__ = version("maxminddb-polars")

__all__ = ["__version__", "lookup_path"]
