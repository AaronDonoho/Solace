pub type Region {
  Domestic
  International
}

pub type Customer {
  Customer(id: Int, name: String, region: Region)
}
