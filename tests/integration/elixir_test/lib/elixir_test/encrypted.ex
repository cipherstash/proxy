defmodule ElixirTest.Encrypted do
  use Ecto.Schema
  import Ecto.Changeset

  @primary_key {:id, :id, autogenerate: false}
  schema "encrypted" do
    field(:plaintext, :string)
    field(:plaintext_date, :date)
    field(:encrypted_text, :string)
    field(:encrypted_bool, :boolean)
    field(:encrypted_int2, :integer)
    field(:encrypted_int4, :integer)
    field(:encrypted_int8, :integer)
    field(:encrypted_float8, :float)
    field(:encrypted_date, :date)
    field(:encrypted_jsonb, :map)
  end

  def changeset(encrypted, attrs) do
    encrypted
    |> cast(attrs, [
      :id,
      :plaintext,
      :plaintext_date,
      :encrypted_text,
      :encrypted_bool,
      :encrypted_int2,
      :encrypted_int4,
      :encrypted_int8,
      :encrypted_float8,
      :encrypted_date,
      :encrypted_jsonb
    ])
  end
end
